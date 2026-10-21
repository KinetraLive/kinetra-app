//! Premultiplied BGRA (what the page capture gives) → the output's pixel format, on the GPU, then
//! read back without ever waiting for the GPU:
//!
//! - **UYVA**: a UYVY pass into a (w/2)×h RGBA8 target (one texel = U Y0 V Y1, BT.709 limited
//!   range, chroma averaged over the pair) and an alpha pass into a w×h R8 target.
//! - **UYVY**: the same color pass composited over the output's background color.
//! - **BGRA**: one pass (straight or premultiplied color, alpha kept).
//!
//! Color is un-premultiplied for [`AlphaMode::Straight`]. Each [`submit`](Converter::submit)
//! draws into the targets and copies them into one of [`SLOTS`] staging slots; [`collect`]
//! (Converter::collect) maps the newest finished slot with `DO_NOT_WAIT`. Shaders are compiled
//! once per converter by d3dcompiler_47 (part of Windows).

use windows::core::{s, PCSTR};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;

use super::gpu::texture;
use super::FrameError;
use crate::core::frame_out::{AlphaMode, PixelFormat};

/// Staging slots: one being written by the GPU, one finished, one spare.
pub const SLOTS: usize = 3;

const HLSL: &str = r#"
Texture2D<float4> src : register(t0);
cbuffer P : register(b0) { float4 bg; uint mode; uint3 pad; };

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}

// mode 0: straight, 1: premultiplied, 2: opaque over bg.
float3 color(float4 c) {
    if (mode == 0) return c.a > 0 ? saturate(c.rgb / c.a) : float3(0, 0, 0);
    if (mode == 1) return c.rgb;
    return c.rgb + bg.rgb * (1 - c.a);
}

static const float3 K = float3(0.2126, 0.7152, 0.0722);

float4 ps_uyvy(float4 pos : SV_Position) : SV_Target {
    int2 p = int2(pos.xy);
    float3 a = color(src.Load(int3(p.x * 2, p.y, 0)));
    float3 b = color(src.Load(int3(p.x * 2 + 1, p.y, 0)));
    float ya = dot(a, K), yb = dot(b, K);
    float3 m = (a + b) * 0.5;
    float ym = (ya + yb) * 0.5;
    float cb = (m.b - ym) / 1.8556, cr = (m.r - ym) / 1.5748;
    return float4(128 + 224 * cb, 16 + 219 * ya, 128 + 224 * cr, 16 + 219 * yb) / 255;
}

float ps_alpha(float4 pos : SV_Position) : SV_Target {
    return src.Load(int3(int2(pos.xy), 0)).a;
}

float4 ps_bgra(float4 pos : SV_Position) : SV_Target {
    float4 c = src.Load(int3(int2(pos.xy), 0));
    return float4(color(c), mode == 2 ? 1 : c.a);
}
"#;

struct Pass {
    target: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    shader: ID3D11PixelShader,
    w: u32,
    h: u32,
    /// Bytes per pixel in the read-back copy.
    bpp: usize,
}

struct Slot {
    staging: Vec<ID3D11Texture2D>,
    /// The submit sequence number while the GPU owns it.
    pending: Option<u64>,
}

pub struct Converter {
    w: u32,
    h: u32,
    format: PixelFormat,
    srv: ID3D11ShaderResourceView,
    vs: ID3D11VertexShader,
    cbuf: ID3D11Buffer,
    passes: Vec<Pass>,
    slots: Vec<Slot>,
    seq: u64,
}

// SAFETY: D3D11 objects are free-threaded; every context call goes through the multithread-
// protected immediate context. A converter is used by one thread at a time.
unsafe impl Send for Converter {}

/// What [`Converter::collect`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collected {
    /// The newest finished frame was copied out (its submit sequence number).
    Frame(u64),
    /// Something is still being drawn and nothing newer than last time is finished.
    Pending,
    /// Nothing was submitted since the last frame.
    Idle,
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob, FrameError> {
    let mut code = None;
    let mut errors = None;
    let r = unsafe {
        D3DCompile(
            HLSL.as_ptr().cast(),
            HLSL.len(),
            s!("frames.hlsl"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = r {
        let msg = errors
            .map(|b| unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    b.GetBufferPointer() as *const u8,
                    b.GetBufferSize(),
                ))
                .into_owned()
            })
            .unwrap_or_default();
        return Err(FrameError::Gpu(format!("shader: {e} {msg}")));
    }
    code.ok_or_else(|| FrameError::Gpu("shader: no code".into()))
}

fn bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

impl Converter {
    /// A converter reading `src` (a w×h BGRA texture bound as a shader resource).
    pub fn new(
        device: &ID3D11Device,
        src: &ID3D11Texture2D,
        w: u32,
        h: u32,
        format: PixelFormat,
        alpha: AlphaMode,
        background: [u8; 3],
    ) -> Result<Self, FrameError> {
        let gpu = |e: windows::core::Error| FrameError::Gpu(e.to_string());
        if w < 2 || h < 1 || !w.is_multiple_of(2) {
            return Err(FrameError::Gpu(format!(
                "unsupported size {w}×{h} (width must be even)"
            )));
        }
        let vs_code = compile(s!("vs"), s!("vs_4_0"))?;
        let mut vs = None;
        unsafe { device.CreateVertexShader(bytes(&vs_code), None, Some(&mut vs)) }.map_err(gpu)?;
        let ps = |entry: PCSTR| -> Result<ID3D11PixelShader, FrameError> {
            let code = compile(entry, s!("ps_4_0"))?;
            let mut p = None;
            unsafe { device.CreatePixelShader(bytes(&code), None, Some(&mut p)) }.map_err(gpu)?;
            p.ok_or_else(|| FrameError::Gpu("no pixel shader".into()))
        };
        let pass = |shader: ID3D11PixelShader,
                    w: u32,
                    h: u32,
                    fmt: DXGI_FORMAT,
                    bpp: usize|
         -> Result<Pass, FrameError> {
            let target = texture(device, w, h, fmt, D3D11_BIND_RENDER_TARGET, false)?;
            let mut rtv = None;
            unsafe { device.CreateRenderTargetView(&target, None, Some(&mut rtv)) }.map_err(gpu)?;
            Ok(Pass {
                target,
                rtv: rtv.ok_or_else(|| FrameError::Gpu("no rtv".into()))?,
                shader,
                w,
                h,
                bpp,
            })
        };
        let passes = match format {
            PixelFormat::Uyva => vec![
                pass(ps(s!("ps_uyvy"))?, w / 2, h, DXGI_FORMAT_R8G8B8A8_UNORM, 4)?,
                pass(ps(s!("ps_alpha"))?, w, h, DXGI_FORMAT_R8_UNORM, 1)?,
            ],
            PixelFormat::Uyvy => vec![pass(
                ps(s!("ps_uyvy"))?,
                w / 2,
                h,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                4,
            )?],
            PixelFormat::Bgra => vec![pass(
                ps(s!("ps_bgra"))?,
                w,
                h,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                4,
            )?],
        };
        let mut slots = Vec::with_capacity(SLOTS);
        for _ in 0..SLOTS {
            let staging = passes
                .iter()
                .map(|p| {
                    let fmt = unsafe {
                        let mut d = D3D11_TEXTURE2D_DESC::default();
                        p.target.GetDesc(&mut d);
                        d.Format
                    };
                    texture(device, p.w, p.h, fmt, D3D11_BIND_FLAG(0), true)
                })
                .collect::<Result<Vec<_>, _>>()?;
            slots.push(Slot {
                staging,
                pending: None,
            });
        }
        let mut srv = None;
        unsafe { device.CreateShaderResourceView(src, None, Some(&mut srv)) }.map_err(gpu)?;
        // Constants: bg (float4), mode (uint), padding.
        let mode: u32 = match (format, alpha) {
            (PixelFormat::Uyvy, _) => 2,
            (_, AlphaMode::Straight) => 0,
            (_, AlphaMode::Premultiplied) => 1,
        };
        let mut consts = [0u32; 8];
        for (i, c) in background.iter().enumerate() {
            consts[i] = (f32::from(*c) / 255.0).to_bits();
        }
        consts[3] = 1f32.to_bits();
        consts[4] = mode;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: 32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: consts.as_ptr().cast(),
            ..Default::default()
        };
        let mut cbuf = None;
        unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut cbuf)) }.map_err(gpu)?;
        Ok(Self {
            w,
            h,
            format,
            srv: srv.ok_or_else(|| FrameError::Gpu("no srv".into()))?,
            vs: vs.ok_or_else(|| FrameError::Gpu("no vertex shader".into()))?,
            cbuf: cbuf.ok_or_else(|| FrameError::Gpu("no constant buffer".into()))?,
            passes,
            slots,
            seq: 0,
        })
    }

    /// Bytes of one read-back frame.
    pub fn frame_bytes(&self) -> usize {
        self.format.frame_bytes(self.w, self.h)
    }

    /// True while a slot is free for [`submit`](Self::submit).
    pub fn can_submit(&self) -> bool {
        self.slots.iter().any(|s| s.pending.is_none())
    }

    /// Converts the source's current contents into a free staging slot (GPU work, queued and
    /// flushed; returns at once). `None` when every slot is still owned by the GPU.
    pub fn submit(&mut self, ctx: &ID3D11DeviceContext) -> Option<u64> {
        let slot = self.slots.iter().position(|s| s.pending.is_none())?;
        self.seq += 1;
        unsafe {
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShaderResources(0, Some(&[Some(self.srv.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.OMSetBlendState(None, None, u32::MAX);
            for (i, p) in self.passes.iter().enumerate() {
                ctx.OMSetRenderTargets(Some(&[Some(p.rtv.clone())]), None);
                ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: p.w as f32,
                    Height: p.h as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
                ctx.PSSetShader(&p.shader, None);
                ctx.Draw(3, 0);
                ctx.CopyResource(&self.slots[slot].staging[i], &p.target);
            }
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
            ctx.Flush();
        }
        self.slots[slot].pending = Some(self.seq);
        Some(self.seq)
    }

    /// Copies the newest finished slot into `out` (`frame_bytes()` long) without waiting for the
    /// GPU; older finished slots are released unread.
    pub fn collect(
        &mut self,
        ctx: &ID3D11DeviceContext,
        out: &mut [u8],
    ) -> Result<Collected, FrameError> {
        let mut order: Vec<(usize, u64)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.pending.map(|q| (i, q)))
            .collect();
        if order.is_empty() {
            return Ok(Collected::Idle);
        }
        order.sort_by_key(|o| std::cmp::Reverse(o.1));
        for &(i, seq) in &order {
            if self.read(ctx, i, out, false)? {
                for s in &mut self.slots {
                    if s.pending.is_some_and(|q| q <= seq) {
                        s.pending = None;
                    }
                }
                return Ok(Collected::Frame(seq));
            }
        }
        Ok(Collected::Pending)
    }

    /// Submits and waits for the result (tests and one-off captures).
    pub fn convert_blocking(
        &mut self,
        ctx: &ID3D11DeviceContext,
        out: &mut [u8],
    ) -> Result<(), FrameError> {
        let slot_seq = self
            .submit(ctx)
            .ok_or_else(|| FrameError::Gpu("no free slot".into()))?;
        let i = self
            .slots
            .iter()
            .position(|s| s.pending == Some(slot_seq))
            .ok_or_else(|| FrameError::Gpu("slot".into()))?;
        self.read(ctx, i, out, true)?;
        self.slots[i].pending = None;
        Ok(())
    }

    /// Maps slot `i`'s staging textures and copies them into `out`. `false` if still drawing.
    fn read(
        &self,
        ctx: &ID3D11DeviceContext,
        i: usize,
        out: &mut [u8],
        wait: bool,
    ) -> Result<bool, FrameError> {
        let flags = if wait {
            0
        } else {
            D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32
        };
        let mut offset = 0usize;
        for (p, staging) in self.passes.iter().zip(&self.slots[i].staging) {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            match unsafe { ctx.Map(staging, 0, D3D11_MAP_READ, flags, Some(&mut m)) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => return Ok(false),
                Err(e) => return Err(FrameError::Gpu(e.to_string())),
            }
            let row = p.w as usize * p.bpp;
            let rows = p.h as usize;
            if m.pData.is_null() || (m.RowPitch as usize) < row || out.len() < offset + row * rows {
                unsafe { ctx.Unmap(staging, 0) };
                return Err(FrameError::Gpu(
                    windows::core::Error::from(E_FAIL).to_string(),
                ));
            }
            // SAFETY: the mapped subresource holds `rows` rows of `RowPitch` bytes.
            let src = unsafe {
                std::slice::from_raw_parts(
                    m.pData as *const u8,
                    m.RowPitch as usize * (rows - 1) + row,
                )
            };
            for y in 0..rows {
                let s = y * m.RowPitch as usize;
                out[offset + y * row..offset + (y + 1) * row].copy_from_slice(&src[s..s + row]);
            }
            unsafe { ctx.Unmap(staging, 0) };
            offset += row * rows;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::gpu::{self, Adapter};

    /// BT.709 limited-range reference on the CPU (premultiplied BGRA in).
    fn reference(
        bgra: &[u8],
        w: usize,
        h: usize,
        format: PixelFormat,
        alpha: AlphaMode,
        bg: [u8; 3],
    ) -> Vec<u8> {
        let color = |px: &[u8]| -> [f64; 3] {
            let (b, g, r, a) = (
                px[0] as f64 / 255.0,
                px[1] as f64 / 255.0,
                px[2] as f64 / 255.0,
                px[3] as f64 / 255.0,
            );
            match (format, alpha) {
                (PixelFormat::Uyvy, _) => {
                    let k = 1.0 - a;
                    [
                        r + bg[0] as f64 / 255.0 * k,
                        g + bg[1] as f64 / 255.0 * k,
                        b + bg[2] as f64 / 255.0 * k,
                    ]
                }
                (_, AlphaMode::Premultiplied) => [r, g, b],
                (_, AlphaMode::Straight) if a > 0.0 => {
                    [(r / a).min(1.0), (g / a).min(1.0), (b / a).min(1.0)]
                }
                _ => [0.0; 3],
            }
        };
        let q = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        let y_of = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let mut out = Vec::new();
        match format {
            PixelFormat::Bgra => {
                for px in bgra.chunks(4) {
                    let c = color(px);
                    out.extend([q(c[2]), q(c[1]), q(c[0]), px[3]]);
                }
            }
            PixelFormat::Uyva | PixelFormat::Uyvy => {
                for y in 0..h {
                    for x in (0..w).step_by(2) {
                        let a = color(&bgra[(y * w + x) * 4..]);
                        let b = color(&bgra[(y * w + x + 1) * 4..]);
                        let (ya, yb) = (y_of(a), y_of(b));
                        let m = [
                            (a[0] + b[0]) / 2.0,
                            (a[1] + b[1]) / 2.0,
                            (a[2] + b[2]) / 2.0,
                        ];
                        let ym = (ya + yb) / 2.0;
                        let (cb, cr) = ((m[2] - ym) / 1.8556, (m[0] - ym) / 1.5748);
                        out.extend([
                            q((128.0 + 224.0 * cb) / 255.0),
                            q((16.0 + 219.0 * ya) / 255.0),
                            q((128.0 + 224.0 * cr) / 255.0),
                            q((16.0 + 219.0 * yb) / 255.0),
                        ]);
                    }
                }
                if format == PixelFormat::Uyva {
                    out.extend(bgra.chunks(4).map(|px| px[3]));
                }
            }
        }
        out
    }

    /// A premultiplied test image: clear, opaque colors, half-transparent red, a gradient.
    fn image(w: usize, h: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let px: [u8; 4] = match (x / 4, y % 4) {
                    (0, _) => [0, 0, 0, 0],
                    (1, _) => [0, 255, 0, 255],
                    (2, _) => [0, 0, 128, 128],
                    (3, _) => [255, 255, 255, 255],
                    _ => {
                        let a = ((x * 37 + y * 11) % 256) as u8;
                        let c = |k: usize| ((k as u32 * a as u32) / 255) as u8;
                        [c(x * 3 % 256), c(y * 7 % 256), c((x + y) % 256), a]
                    }
                };
                v[(y * w + x) * 4..][..4].copy_from_slice(&px);
            }
        }
        v
    }

    fn run(format: PixelFormat, alpha: AlphaMode, bg: [u8; 3]) {
        let (w, h) = (64usize, 8usize);
        let g = gpu::create(Adapter::Warp).expect("WARP device");
        let pixels = image(w, h);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w as u32,
            Height: h as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: (w * 4) as u32,
            SysMemSlicePitch: 0,
        };
        let mut src = None;
        unsafe { g.device.CreateTexture2D(&desc, Some(&init), Some(&mut src)) }.unwrap();
        let src = src.unwrap();
        let mut c = Converter::new(&g.device, &src, w as u32, h as u32, format, alpha, bg).unwrap();
        let mut out = vec![0u8; c.frame_bytes()];
        c.convert_blocking(&g.context, &mut out).unwrap();
        let want = reference(&pixels, w, h, format, alpha, bg);
        assert_eq!(out.len(), want.len());
        let worst = out
            .iter()
            .zip(&want)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "{format:?} {alpha:?}: off by {worst}");
    }

    #[test]
    fn uyva_straight_matches_the_reference() {
        run(PixelFormat::Uyva, AlphaMode::Straight, [0, 0, 0]);
    }

    #[test]
    fn uyva_premultiplied_matches_the_reference() {
        run(PixelFormat::Uyva, AlphaMode::Premultiplied, [0, 0, 0]);
    }

    #[test]
    fn uyvy_over_a_background_matches_the_reference() {
        run(PixelFormat::Uyvy, AlphaMode::Straight, [0, 177, 64]);
    }

    #[test]
    fn bgra_matches_the_reference() {
        run(PixelFormat::Bgra, AlphaMode::Straight, [0, 0, 0]);
    }

    #[test]
    fn known_values() {
        // Opaque green → Y 173, Cb 42, Cr 26 (BT.709 limited); half red straight → Y 63.
        let ref_green = reference(
            &[0, 255, 0, 255, 0, 255, 0, 255],
            2,
            1,
            PixelFormat::Uyva,
            AlphaMode::Straight,
            [0; 3],
        );
        assert_eq!(&ref_green[..4], &[42, 173, 26, 173]);
        let half_red = reference(
            &[0, 0, 128, 128, 0, 0, 128, 128],
            2,
            1,
            PixelFormat::Uyva,
            AlphaMode::Straight,
            [0; 3],
        );
        assert_eq!(half_red[1], 63);
        assert_eq!(&half_red[4..], &[128, 128]);
    }

    #[test]
    fn collect_never_waits_and_releases_slots() {
        let g = gpu::create(Adapter::Warp).unwrap();
        let src = texture(
            &g.device,
            16,
            4,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_BIND_SHADER_RESOURCE,
            false,
        )
        .unwrap();
        let mut c = Converter::new(
            &g.device,
            &src,
            16,
            4,
            PixelFormat::Uyva,
            AlphaMode::Straight,
            [0; 3],
        )
        .unwrap();
        let mut out = vec![0u8; c.frame_bytes()];
        assert_eq!(c.collect(&g.context, &mut out).unwrap(), Collected::Idle);
        for _ in 0..SLOTS {
            assert!(c.submit(&g.context).is_some());
        }
        assert!(
            !c.can_submit() && c.submit(&g.context).is_none(),
            "all slots owned by the GPU"
        );
        // Eventually the newest one is collected and every slot is free again.
        let mut got = None;
        for _ in 0..1000 {
            match c.collect(&g.context, &mut out).unwrap() {
                Collected::Frame(seq) => {
                    got = Some(seq);
                    break;
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
        assert!(got.is_some());
        assert!(c.can_submit());
    }
}
