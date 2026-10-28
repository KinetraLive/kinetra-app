//! Baseline JPEG with Windows Imaging Component (the remote preview's pictures, protocol 2
//! §6.2): 24-bit BGR in, sRGB JPEG out, encoded into a fixed buffer so a picture over the
//! server's limit fails instead of growing (the caller then steps the quality down).
//!
//! One [`Jpeg`] per thread (WIC's factory is created on the calling thread's COM apartment).

use windows::core::GUID;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPBAG2};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STREAM_SEEK_CUR,
};
use windows::Win32::System::Variant::VARIANT;

use super::FrameError;

fn wic(e: windows::core::Error) -> FrameError {
    FrameError::Gpu(format!("JPEG: {e}"))
}

pub struct Jpeg {
    factory: IWICImagingFactory,
    /// The encoder writes here (the server's limit); the picture is its first `len` bytes.
    buf: Vec<u8>,
}

impl Jpeg {
    /// A JPEG encoder for pictures of at most `max_bytes`.
    pub fn new(max_bytes: usize) -> Result<Self, FrameError> {
        // SAFETY: COM on this thread (an apartment that's already set up is fine: S_FALSE).
        let factory = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(wic)?
        };
        Ok(Self {
            factory,
            buf: vec![0; max_bytes.max(1024)],
        })
    }

    /// Encodes a w×h 24-bit BGR picture (`stride` bytes per row) at `quality` (0–1). `Ok(None)`
    /// when it doesn't fit in the buffer.
    // WIC writes the picture into the buffer it is given: it must be borrowed mutably.
    #[allow(clippy::unnecessary_mut_passed)]
    pub fn encode(
        &mut self,
        w: u32,
        h: u32,
        stride: u32,
        bgr: &[u8],
        quality: f32,
    ) -> Result<Option<&[u8]>, FrameError> {
        if bgr.len() < stride as usize * h as usize {
            return Err(FrameError::Gpu("JPEG: short picture".into()));
        }
        let len = unsafe {
            let stream = self.factory.CreateStream().map_err(wic)?;
            stream.InitializeFromMemory(&mut self.buf).map_err(wic)?;
            let encoder = self
                .factory
                .CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())
                .map_err(wic)?;
            encoder
                .Initialize(&stream, WICBitmapEncoderNoCache)
                .map_err(wic)?;
            let mut frame: Option<IWICBitmapFrameEncode> = None;
            let mut bag: Option<IPropertyBag2> = None;
            encoder.CreateNewFrame(&mut frame, &mut bag).map_err(wic)?;
            let frame = frame.ok_or_else(|| FrameError::Gpu("JPEG: no frame".into()))?;
            let bag = bag.ok_or_else(|| FrameError::Gpu("JPEG: no options".into()))?;
            let mut name: Vec<u16> = "ImageQuality\0".encode_utf16().collect();
            let opt = PROPBAG2 {
                pstrName: windows::core::PWSTR(name.as_mut_ptr()),
                ..Default::default()
            };
            let value = VARIANT::from(quality.clamp(0.0, 1.0));
            bag.Write(1, &opt, &value).map_err(wic)?;
            frame.Initialize(&bag).map_err(wic)?;
            frame.SetSize(w, h).map_err(wic)?;
            let mut format: GUID = GUID_WICPixelFormat24bppBGR;
            frame.SetPixelFormat(&mut format).map_err(wic)?;
            if format != GUID_WICPixelFormat24bppBGR {
                return Err(FrameError::Gpu("JPEG: pixel format refused".into()));
            }
            let bytes = &bgr[..stride as usize * h as usize];
            // A full buffer fails here or at a commit: the picture is too large.
            if frame.WritePixels(h, stride, bytes).is_err()
                || frame.Commit().is_err()
                || encoder.Commit().is_err()
            {
                return Ok(None);
            }
            let mut pos = 0u64;
            stream
                .Seek(0, STREAM_SEEK_CUR, Some(&mut pos))
                .map_err(wic)?;
            pos as usize
        };
        // A memory stream that filled up may report success with a cut picture: a full buffer or
        // a picture without its end marker is too large.
        if len < 4 || len >= self.buf.len() || self.buf[len - 2..len] != [0xff, 0xd9] {
            return Ok(None);
        }
        Ok(Some(&self.buf[..len]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes with WIC: (width, height, the BGR pixel at the center).
    fn decode(jpeg: &[u8]) -> (u32, u32, [u8; 3]) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let f: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let s = f.CreateStream().unwrap();
            let copy = jpeg.to_vec();
            s.InitializeFromMemory(&copy).unwrap();
            let d = f
                .CreateDecoderFromStream(&s, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
                .unwrap();
            let fr = d.GetFrame(0).unwrap();
            let (mut w, mut h) = (0, 0);
            fr.GetSize(&mut w, &mut h).unwrap();
            let conv = f.CreateFormatConverter().unwrap();
            conv.Initialize(
                &fr,
                &GUID_WICPixelFormat24bppBGR,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .unwrap();
            let mut px = vec![0u8; (w * h * 3) as usize];
            conv.CopyPixels(std::ptr::null(), w * 3, &mut px).unwrap();
            let c = ((h / 2 * w + w / 2) * 3) as usize;
            (w, h, [px[c], px[c + 1], px[c + 2]])
        }
    }

    #[test]
    fn encodes_a_picture_that_decodes_back() {
        let (w, h) = (64u32, 36u32);
        // A solid orange (B, G, R).
        let bgr: Vec<u8> = (0..w * h).flat_map(|_| [0x20, 0x80, 0xf0]).collect();
        let mut j = Jpeg::new(256 * 1024).unwrap();
        let out = j.encode(w, h, w * 3, &bgr, 0.7).unwrap().unwrap().to_vec();
        assert_eq!(&out[..2], &[0xff, 0xd8], "a JPEG");
        assert_eq!(&out[out.len() - 2..], &[0xff, 0xd9]);
        let (dw, dh, c) = decode(&out);
        assert_eq!((dw, dh), (w, h));
        for (got, want) in c.iter().zip([0x20u8, 0x80, 0xf0]) {
            assert!(got.abs_diff(want) <= 6, "{c:?}");
        }
        // Quality changes the size.
        let noise: Vec<u8> = (0..w * h * 3).map(|i| (i * 7919 % 251) as u8).collect();
        let hi = j.encode(w, h, w * 3, &noise, 0.95).unwrap().unwrap().len();
        let lo = j.encode(w, h, w * 3, &noise, 0.3).unwrap().unwrap().len();
        assert!(lo < hi, "{lo} < {hi}");
    }

    #[test]
    fn a_picture_over_the_limit_is_refused() {
        let (w, h) = (256u32, 256u32);
        let noise: Vec<u8> = (0..w * h * 3).map(|i| (i * 7919 % 251) as u8).collect();
        let mut j = Jpeg::new(2048).unwrap();
        assert!(j.encode(w, h, w * 3, &noise, 0.95).unwrap().is_none());
    }
}
