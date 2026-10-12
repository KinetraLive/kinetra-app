//! Media file probing: our own container readers, no external tools and no crates. Headers only:
//! at most [`HEAD_MAX`] bytes from the start, plus an MP4's `moov` box wherever it is (at most
//! [`MOOV_MAX`]). A 4 GB file costs a few reads.
//!
//! - **MP4 / M4A / MOV** (ISO BMFF): `mvhd` duration, `tkhd` size, `stsd` codec (`avcC` profile
//!   and level for H.264; `hvc1`/`hev1`, `av01`, `vp09`, `vp08`, ProRes, `mp4a`, `Opus`, …), fps
//!   from `stts` with the `mdhd` timescale, fast start (`moov` before `mdat`). Alpha: ProRes 4444
//!   with depth 32, or HEVC whose sample entry's compressor name mentions alpha (what Apple's
//!   encoders write; HEVC alpha in a VPS extension is not parsed).
//! - **WebM / Matroska** (EBML): Duration × TimecodeScale, CodecID, PixelWidth/Height,
//!   AlphaMode, DefaultDuration (fps).
//! - **MP3**: ID3v2 skipped, first frame header, Xing/Info or VBRI frame count; otherwise a CBR
//!   estimate from the file size.
//! - **WAV**: RIFF `fmt ` / `data`. **AAC** (ADTS): frames walked inside the head, the rest
//!   estimated from the average frame size.
//! - **Images**: size from PNG / JPEG / GIF / WebP headers (SVG: none).
//!
//! Truncated or corrupt files give an `unreadable` issue, never a panic. Results are cached in
//! memory by (path, size, mtime) ([`info_cached`]); nothing is written into the project.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use crate::core::media_support;
use crate::core::model::{
    MediaInfo, MediaKind, ASSET_EXTENSIONS, MEDIA_AUDIO_EXTENSIONS, MEDIA_VIDEO_EXTENSIONS,
};

/// Bytes read from the start of a file.
pub const HEAD_MAX: usize = 1024 * 1024;
/// Largest `moov` box read (long files with big sample tables stay well below this).
pub const MOOV_MAX: u64 = 64 * 1024 * 1024;
/// Most entries kept in the probe cache (it is cleared when full).
const CACHE_MAX: usize = 4096;

/// What a file of this (lower-case) extension is, if it is a media library type.
pub fn kind_of_ext(ext: &str) -> Option<MediaKind> {
    let ext = ext.to_ascii_lowercase();
    if MEDIA_VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else if MEDIA_AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else if ASSET_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Image)
    } else {
        None
    }
}

/// The media kind of an asset name (by extension).
pub fn kind_of_name(name: &str) -> Option<MediaKind> {
    kind_of_ext(name.rsplit_once('.')?.1)
}

/// The container a (lower-case) extension stands for.
fn container_of_ext(ext: &str) -> &'static str {
    match ext {
        "mp4" | "m4v" | "m4a" => "mp4",
        "mov" => "mov",
        "webm" => "webm",
        "mp3" => "mp3",
        "wav" => "wav",
        "aac" => "aac",
        "png" => "png",
        "jpg" | "jpeg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        "svg" => "svg",
        _ => "unknown",
    }
}

#[derive(Debug)]
struct Bad;

impl From<io::Error> for Bad {
    fn from(_: io::Error) -> Self {
        Bad
    }
}

type R<T> = Result<T, Bad>;

/// A blank result for a file of this extension.
fn blank(ext: &str) -> MediaInfo {
    MediaInfo {
        kind: kind_of_ext(ext).unwrap_or(MediaKind::Video),
        container: container_of_ext(ext).to_string(),
        duration_ms: None,
        width: None,
        height: None,
        fps: None,
        video_codec: None,
        audio_codec: None,
        alpha: false,
        fast_start: None,
        issues: Vec::new(),
    }
}

/// Probes a file: headers only. `name` is the file's name in `assets/` (for the issues'
/// convert commands). Never fails: problems become an `unreadable` issue.
pub fn probe_file(path: &Path, name: &str) -> MediaInfo {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let probed = fs::File::open(path).and_then(|mut f| {
        let len = f.metadata()?.len();
        Ok(probe_reader(&mut f, len, &ext))
    });
    let info = match probed {
        Ok(Ok(info)) => Some(info),
        _ => None,
    };
    finish(info, &ext, name)
}

/// Probes bytes in memory (tests, and files Kinetra writes itself).
pub fn probe_bytes(bytes: &[u8], name: &str) -> MediaInfo {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mut c = io::Cursor::new(bytes);
    let info = probe_reader(&mut c, bytes.len() as u64, &ext).ok();
    finish(info, &ext, name)
}

fn finish(info: Option<MediaInfo>, ext: &str, name: &str) -> MediaInfo {
    let mut info = match info {
        Some(i) => i,
        None => {
            let mut b = blank(ext);
            b.issues.push(media_support::unreadable());
            return b;
        }
    };
    info.issues = media_support::issues(&info, name);
    info
}

fn probe_reader<F: Read + Seek>(f: &mut F, len: u64, ext: &str) -> R<MediaInfo> {
    let head = read_head(f, len)?;
    let mut info = blank(ext);
    match container_of_ext(ext) {
        "mp4" | "mov" => mp4::probe(f, len, &head, &mut info)?,
        "webm" => ebml::probe(&head, len <= head.len() as u64, &mut info)?,
        "mp3" => audio::mp3(&head, len, &mut info)?,
        "wav" => audio::wav(f, len, &head, &mut info)?,
        "aac" => audio::adts(&head, len, &mut info)?,
        "png" | "jpg" | "gif" | "webp" => image::probe(&head, &mut info)?,
        "svg" => {}
        _ => return Err(Bad),
    }
    // A video container holding only sound plays as audio (an .m4a renamed .mp4, say).
    if info.kind == MediaKind::Video && info.video_codec.is_none() && info.audio_codec.is_some() {
        info.kind = MediaKind::Audio;
    }
    Ok(info)
}

fn read_head<F: Read + Seek>(f: &mut F, len: u64) -> R<Vec<u8>> {
    let n = usize::try_from(len.min(HEAD_MAX as u64)).map_err(|_| Bad)?;
    let mut head = vec![0u8; n];
    f.seek(SeekFrom::Start(0))?;
    f.read_exact(&mut head)?;
    Ok(head)
}

fn be16(b: &[u8], at: usize) -> R<u16> {
    let s = b.get(at..at + 2).ok_or(Bad)?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

fn be32(b: &[u8], at: usize) -> R<u32> {
    let s = b.get(at..at + 4).ok_or(Bad)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be64(b: &[u8], at: usize) -> R<u64> {
    let s = b.get(at..at + 8).ok_or(Bad)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Ok(u64::from_be_bytes(a))
}

fn le16(b: &[u8], at: usize) -> R<u16> {
    let s = b.get(at..at + 2).ok_or(Bad)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn le32(b: &[u8], at: usize) -> R<u32> {
    let s = b.get(at..at + 4).ok_or(Bad)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `num / den` seconds in ms, rounded.
fn ms(num: u64, den: u64) -> Option<u64> {
    if den == 0 {
        return None;
    }
    let v = (u128::from(num) * 1000 + u128::from(den) / 2) / u128::from(den);
    u64::try_from(v).ok()
}

/// Frames per second rounded to 3 decimals; `None` outside 1..=1000.
fn fps(v: f64) -> Option<f64> {
    (v.is_finite() && (1.0..=1000.0).contains(&v)).then(|| (v * 1000.0).round() / 1000.0)
}

// ---- ISO BMFF (MP4, M4A, MOV) ------------------------------------------------------------------

mod mp4 {
    use super::*;

    /// One box: type, payload range in the buffer (or file).
    struct Bx {
        kind: [u8; 4],
        start: u64,
        end: u64,
    }

    /// Box header at `at` (`len` = end of the enclosing range). `None` at the end.
    fn header(b: &[u8], at: usize, len: usize) -> R<Option<Bx>> {
        if at + 8 > len {
            return Ok(None);
        }
        let size = u64::from(be32(b, at)?);
        let mut kind = [0u8; 4];
        kind.copy_from_slice(b.get(at + 4..at + 8).ok_or(Bad)?);
        let (hdr, size) = match size {
            0 => (8, (len - at) as u64),
            1 => (16, be64(b, at + 8)?),
            s => (8, s),
        };
        if size < hdr || at as u64 + size > len as u64 {
            return Err(Bad);
        }
        Ok(Some(Bx {
            kind,
            start: at as u64 + hdr,
            end: at as u64 + size,
        }))
    }

    /// Children of a box payload `b[start..end]`.
    fn children(b: &[u8], start: usize, end: usize) -> R<Vec<Bx>> {
        let mut out = Vec::new();
        let mut at = start;
        while let Some(bx) = header(b, at, end)? {
            at = bx.end as usize;
            out.push(bx);
        }
        Ok(out)
    }

    fn find<'a>(boxes: &'a [Bx], kind: &[u8; 4]) -> Option<&'a Bx> {
        boxes.iter().find(|b| &b.kind == kind)
    }

    /// Most fragments (`moof`) read for a fragmented file's duration.
    const MOOF_MAX: usize = 20_000;
    /// Largest `moof` read.
    const MOOF_BYTES_MAX: u64 = 4 * 1024 * 1024;

    #[derive(Default)]
    struct Track {
        id: u32,
        handler: [u8; 4],
        width: Option<u32>,
        height: Option<u32>,
        codec: Option<String>,
        alpha: bool,
        timescale: u64,
        duration: u64,
        samples: u64,
        sample_time: u64,
        /// Fragmented files: `trex` default sample duration, and the fragments' totals.
        trex_duration: u64,
        frag_samples: u64,
        frag_time: u64,
    }

    pub(super) fn probe<F: Read + Seek>(
        f: &mut F,
        len: u64,
        head: &[u8],
        info: &mut MediaInfo,
    ) -> R<()> {
        // Top level: walk headers through the file (seeking over big boxes).
        let mut at = 0u64;
        let mut moov: Option<(u64, u64)> = None;
        let mut mdat_at: Option<u64> = None;
        let mut brand = [0u8; 4];
        let mut first = true;
        let mut moofs: Vec<(u64, u64)> = Vec::new();
        while at + 8 <= len {
            let mut h = [0u8; 16];
            let n = if at + 16 <= len { 16 } else { 8 };
            f.seek(SeekFrom::Start(at))?;
            f.read_exact(&mut h[..n])?;
            let size32 = u64::from(be32(&h, 0)?);
            let kind = [h[4], h[5], h[6], h[7]];
            let (hdr, size) = match size32 {
                0 => (8, len - at),
                1 if n == 16 => (16, be64(&h, 8)?),
                1 => return Err(Bad),
                s => (8, s),
            };
            if first {
                // An ISO file starts with ftyp (or a QuickTime atom).
                if !matches!(
                    &kind,
                    b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip"
                ) {
                    return Err(Bad);
                }
                first = false;
            }
            if size < hdr {
                return Err(Bad);
            }
            let end = at.saturating_add(size).min(len);
            match &kind {
                b"ftyp" => {
                    if let Some(s) = head.get((at + hdr) as usize..(at + hdr + 4) as usize) {
                        brand.copy_from_slice(s);
                    }
                }
                b"moov" if moov.is_none() => moov = Some((at + hdr, end)),
                b"mdat" if mdat_at.is_none() => mdat_at = Some(at),
                b"moof" if moofs.len() < MOOF_MAX => moofs.push((at + hdr, end)),
                _ => {}
            }
            if moofs.len() >= MOOF_MAX {
                break;
            }
            at = at.saturating_add(size);
        }
        if &brand == b"qt  " {
            info.container = "mov".into();
        }
        let (ms_start, ms_end) = moov.ok_or(Bad)?;
        if ms_end - ms_start > MOOV_MAX {
            return Err(Bad);
        }
        info.fast_start = Some(match mdat_at {
            Some(m) => ms_start < m,
            None => true,
        });
        let buf = if (ms_end as usize) <= head.len() {
            head[ms_start as usize..ms_end as usize].to_vec()
        } else {
            let mut v = vec![0u8; (ms_end - ms_start) as usize];
            f.seek(SeekFrom::Start(ms_start))?;
            f.read_exact(&mut v)?;
            v
        };
        let mut tracks = moov_box(&buf, info)?;
        if !moofs.is_empty() {
            fragments(f, &moofs, &mut tracks);
            if info.duration_ms.is_none() {
                info.duration_ms = tracks
                    .iter()
                    .filter_map(|t| ms(t.frag_time, t.timescale))
                    .filter(|d| *d > 0)
                    .max();
            }
            if let Some(v) = tracks.iter().find(|t| &t.handler == b"vide") {
                if info.fps.is_none() && v.frag_samples > 0 && v.frag_time > 0 {
                    info.fps = fps(v.frag_samples as f64 * v.timescale as f64 / v.frag_time as f64);
                }
            }
        }
        Ok(())
    }

    /// Sums sample durations per track over the fragments (`traf`: `tfhd` defaults, `trun`).
    fn fragments<F: Read + Seek>(f: &mut F, moofs: &[(u64, u64)], tracks: &mut [Track]) {
        let mut buf = Vec::new();
        for &(start, end) in moofs {
            let n = end.saturating_sub(start);
            if n > MOOF_BYTES_MAX {
                continue;
            }
            buf.resize(n as usize, 0);
            if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut buf).is_err() {
                return;
            }
            let Ok(kids) = children(&buf, 0, buf.len()) else {
                continue;
            };
            for traf in kids.iter().filter(|k| &k.kind == b"traf") {
                let _ = traf_box(&buf, traf, tracks);
            }
        }
    }

    fn traf_box(b: &[u8], traf: &Bx, tracks: &mut [Track]) -> R<()> {
        let kids = children(b, traf.start as usize, traf.end as usize)?;
        let tfhd = find(&kids, b"tfhd").ok_or(Bad)?;
        let s = tfhd.start as usize;
        let flags = be32(b, s)? & 0x00FF_FFFF;
        let id = be32(b, s + 4)?;
        let mut at = s + 8;
        if flags & 0x01 != 0 {
            at += 8;
        }
        if flags & 0x02 != 0 {
            at += 4;
        }
        let mut default_dur = if flags & 0x08 != 0 {
            u64::from(be32(b, at)?)
        } else {
            0
        };
        let Some(t) = tracks.iter_mut().find(|t| t.id == id) else {
            return Ok(());
        };
        if default_dur == 0 {
            default_dur = t.trex_duration;
        }
        for trun in kids.iter().filter(|k| &k.kind == b"trun") {
            let s = trun.start as usize;
            let flags = be32(b, s)? & 0x00FF_FFFF;
            let count = u64::from(be32(b, s + 4)?);
            let mut at = s + 8;
            if flags & 0x01 != 0 {
                at += 4;
            }
            if flags & 0x04 != 0 {
                at += 4;
            }
            let per = [0x100, 0x200, 0x400, 0x800]
                .iter()
                .filter(|f| flags & **f != 0)
                .count()
                * 4;
            if flags & 0x100 != 0 {
                for i in 0..count.min(1_000_000) as usize {
                    t.frag_time += u64::from(be32(b, at + i * per)?);
                }
            } else {
                t.frag_time += count * default_dur;
            }
            t.frag_samples += count;
        }
        Ok(())
    }

    fn moov_box(b: &[u8], info: &mut MediaInfo) -> R<Vec<Track>> {
        let kids = children(b, 0, b.len())?;
        let mut duration = None;
        if let Some(mvhd) = find(&kids, b"mvhd") {
            let s = mvhd.start as usize;
            let (ts, d) = if b.get(s).copied() == Some(1) {
                (u64::from(be32(b, s + 20)?), be64(b, s + 24)?)
            } else {
                (u64::from(be32(b, s + 12)?), u64::from(be32(b, s + 16)?))
            };
            if d != u64::MAX && d != u64::from(u32::MAX) {
                duration = ms(d, ts).filter(|v| *v > 0);
            }
        }
        let mut tracks = Vec::new();
        for t in kids.iter().filter(|k| &k.kind == b"trak") {
            if let Ok(t) = trak(b, t) {
                tracks.push(t);
            }
        }
        if duration.is_none() {
            duration = tracks
                .iter()
                .filter_map(|t| ms(t.duration, t.timescale))
                .filter(|d| *d > 0)
                .max();
        }
        info.duration_ms = duration;
        let video = tracks.iter().find(|t| &t.handler == b"vide");
        let audio = tracks.iter().find(|t| &t.handler == b"soun");
        if let Some(v) = video {
            info.video_codec = v.codec.clone();
            info.width = v.width;
            info.height = v.height;
            info.alpha = v.alpha;
            if v.samples > 0 && v.sample_time > 0 {
                info.fps = fps(v.samples as f64 * v.timescale as f64 / v.sample_time as f64);
            }
        }
        if let Some(a) = audio {
            info.audio_codec = a.codec.clone();
        }
        // Fragmented files: per-track default sample durations (`mvex` / `trex`).
        if let Some(mvex) = find(&kids, b"mvex") {
            for trex in children(b, mvex.start as usize, mvex.end as usize)?
                .iter()
                .filter(|k| &k.kind == b"trex")
            {
                let s = trex.start as usize;
                let id = be32(b, s + 4)?;
                let d = u64::from(be32(b, s + 12)?);
                if let Some(t) = tracks.iter_mut().find(|t| t.id == id) {
                    t.trex_duration = d;
                }
            }
        }
        Ok(tracks)
    }

    fn trak(b: &[u8], t: &Bx) -> R<Track> {
        let mut out = Track::default();
        let kids = children(b, t.start as usize, t.end as usize)?;
        if let Some(tkhd) = find(&kids, b"tkhd") {
            let s = tkhd.start as usize;
            let v1 = b.get(s).copied() == Some(1);
            out.id = be32(b, s + if v1 { 20 } else { 12 })?;
            let off = if v1 { 88 } else { 76 };
            let w = be32(b, s + off)? >> 16;
            let h = be32(b, s + off + 4)? >> 16;
            if w > 0 && h > 0 {
                out.width = Some(w);
                out.height = Some(h);
            }
        }
        let mdia = find(&kids, b"mdia").ok_or(Bad)?;
        let mk = children(b, mdia.start as usize, mdia.end as usize)?;
        if let Some(mdhd) = find(&mk, b"mdhd") {
            let s = mdhd.start as usize;
            if b.get(s).copied() == Some(1) {
                out.timescale = u64::from(be32(b, s + 20)?);
                out.duration = be64(b, s + 24)?;
            } else {
                out.timescale = u64::from(be32(b, s + 12)?);
                out.duration = u64::from(be32(b, s + 16)?);
            }
        }
        if let Some(hdlr) = find(&mk, b"hdlr") {
            let s = hdlr.start as usize + 8;
            out.handler.copy_from_slice(b.get(s..s + 4).ok_or(Bad)?);
        }
        let minf = find(&mk, b"minf").ok_or(Bad)?;
        let nk = children(b, minf.start as usize, minf.end as usize)?;
        let stbl = find(&nk, b"stbl").ok_or(Bad)?;
        let sk = children(b, stbl.start as usize, stbl.end as usize)?;
        if let Some(stsd) = find(&sk, b"stsd") {
            sample_entry(b, stsd, &mut out)?;
        }
        if let Some(stts) = find(&sk, b"stts") {
            let s = stts.start as usize;
            let n = be32(b, s + 4)? as usize;
            for i in 0..n.min(100_000) {
                let Ok(count) = be32(b, s + 8 + i * 8) else {
                    break;
                };
                let delta = be32(b, s + 12 + i * 8)?;
                out.samples += u64::from(count);
                out.sample_time += u64::from(count) * u64::from(delta);
            }
        }
        Ok(out)
    }

    fn sample_entry(b: &[u8], stsd: &Bx, t: &mut Track) -> R<()> {
        let s = stsd.start as usize + 8;
        let e = header(b, s, stsd.end as usize)?.ok_or(Bad)?;
        let es = e.start as usize;
        let fourcc = e.kind;
        let video = &t.handler == b"vide";
        let name = match &fourcc {
            b"avc1" | b"avc3" => "h264",
            b"hvc1" | b"hev1" | b"dvh1" | b"dvhe" => "hevc",
            b"av01" => "av1",
            b"vp09" => "vp9",
            b"vp08" => "vp8",
            b"mp4v" => "mpeg4",
            b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => "prores",
            b"mp4a" => "aac",
            b"Opus" => "opus",
            b"ac-3" => "ac3",
            b"ec-3" => "eac3",
            b"alac" => "alac",
            b"fLaC" => "flac",
            b".mp3" => "mp3",
            b"lpcm" | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" | b"raw " => "pcm",
            _ => "",
        };
        let mut codec = if name.is_empty() {
            String::from_utf8_lossy(&fourcc).trim().to_ascii_lowercase()
        } else {
            name.to_string()
        };
        if video && e.end as usize >= es + 78 {
            let w = u32::from(be16(b, es + 24)?);
            let h = u32::from(be16(b, es + 26)?);
            if t.width.is_none() && w > 0 && h > 0 {
                t.width = Some(w);
                t.height = Some(h);
            }
            let cn_len = usize::from(b.get(es + 42).copied().unwrap_or(0)).min(31);
            let compressor = b.get(es + 43..es + 43 + cn_len).unwrap_or(&[]);
            let compressor = String::from_utf8_lossy(compressor).to_ascii_lowercase();
            let depth = be16(b, es + 74)?;
            match name {
                "prores" => t.alpha = matches!(&fourcc, b"ap4h" | b"ap4x") && depth == 32,
                "hevc" => t.alpha = compressor.contains("alpha"),
                _ => {}
            }
            if name == "h264" {
                let kids = children(b, es + 78, e.end as usize).unwrap_or_default();
                if let Some(avcc) = find(&kids, b"avcC") {
                    let a = avcc.start as usize;
                    if let (Some(&p), Some(&l)) = (b.get(a + 1), b.get(a + 3)) {
                        let profile = match p {
                            66 => "Baseline",
                            77 => "Main",
                            88 => "Extended",
                            100 => "High",
                            110 => "High 10",
                            122 => "High 4:2:2",
                            244 => "High 4:4:4",
                            _ => "",
                        };
                        if !profile.is_empty() {
                            codec = format!("h264 {profile} {}.{}", l / 10, l % 10);
                        }
                    }
                }
            }
        }
        t.codec = Some(codec);
        Ok(())
    }
}

// ---- EBML (WebM, Matroska) ---------------------------------------------------------------------

mod ebml {
    use super::*;

    const EBML: u32 = 0x1A45_DFA3;
    const SEGMENT: u32 = 0x1853_8067;
    const INFO: u32 = 0x1549_A966;
    const TIMECODE_SCALE: u32 = 0x2A_D7B1;
    const DURATION: u32 = 0x4489;
    const TRACKS: u32 = 0x1654_AE6B;
    const TRACK_ENTRY: u32 = 0xAE;
    const TRACK_TYPE: u32 = 0x83;
    const CODEC_ID: u32 = 0x86;
    const DEFAULT_DURATION: u32 = 0x23_E383;
    const VIDEO: u32 = 0xE0;
    const PIXEL_WIDTH: u32 = 0xB0;
    const PIXEL_HEIGHT: u32 = 0xBA;
    const ALPHA_MODE: u32 = 0x53C0;
    const CLUSTER: u32 = 0x1F43_B675;
    const TIMECODE: u32 = 0xE7;
    const SIMPLE_BLOCK: u32 = 0xA3;
    const BLOCK_GROUP: u32 = 0xA0;
    const BLOCK: u32 = 0xA1;

    /// A (Simple)Block's timecode relative to its cluster.
    fn block_tc(b: &[u8], el: &El) -> Option<i64> {
        let (_, n) = size(b, el.start).ok()?;
        let s = b.get(el.start + n..el.start + n + 2)?;
        Some(i64::from(i16::from_be_bytes([s[0], s[1]])))
    }

    /// The latest block time (in timecode units) from `start` to `end`, clusters of known or
    /// unknown size alike (a cluster met inside the walk starts a new base).
    fn last_block_time(b: &[u8], start: usize, end: usize) -> Option<u64> {
        let mut pos = start;
        let mut base = 0i64;
        let mut best: Option<i64> = None;
        let mut steps = 0usize;
        'outer: loop {
            steps += 1;
            if steps > 100_000 {
                break;
            }
            for el in elements(b, pos, end) {
                match el.id {
                    CLUSTER => {
                        pos = el.start;
                        base = 0;
                        continue 'outer;
                    }
                    TIMECODE => {
                        base = uint(b, &el)
                            .and_then(|v| i64::try_from(v).ok())
                            .unwrap_or(0)
                    }
                    SIMPLE_BLOCK => {
                        if let Some(r) = block_tc(b, &el) {
                            best = Some(best.map_or(base + r, |x| x.max(base + r)));
                        }
                    }
                    BLOCK_GROUP => {
                        for c in elements(b, el.start, el.end)
                            .iter()
                            .filter(|c| c.id == BLOCK)
                        {
                            if let Some(r) = block_tc(b, c) {
                                best = Some(best.map_or(base + r, |x| x.max(base + r)));
                            }
                        }
                    }
                    _ => {}
                }
            }
            break;
        }
        best.and_then(|v| u64::try_from(v).ok())
    }

    /// Element id at `at` (marker bits kept) and its length.
    fn id(b: &[u8], at: usize) -> R<(u32, usize)> {
        let first = *b.get(at).ok_or(Bad)?;
        let n = first.leading_zeros() as usize + 1;
        if n > 4 {
            return Err(Bad);
        }
        let mut v = 0u32;
        for i in 0..n {
            v = (v << 8) | u32::from(*b.get(at + i).ok_or(Bad)?);
        }
        Ok((v, n))
    }

    /// Size vint at `at`: (value or `None` for "unknown", length).
    fn size(b: &[u8], at: usize) -> R<(Option<u64>, usize)> {
        let first = *b.get(at).ok_or(Bad)?;
        let n = first.leading_zeros() as usize + 1;
        if n > 8 {
            return Err(Bad);
        }
        let mut v = u64::from(first) & (0xFF >> n);
        let mut all_ones = v == (0xFF >> n);
        for i in 1..n {
            let byte = *b.get(at + i).ok_or(Bad)?;
            all_ones &= byte == 0xFF;
            v = (v << 8) | u64::from(byte);
        }
        Ok((if all_ones { None } else { Some(v) }, n))
    }

    struct El {
        id: u32,
        start: usize,
        /// Clamped to the buffer (a truncated head ends early).
        end: usize,
    }

    /// Elements in `b[start..end]`; stops quietly at a truncated element.
    fn elements(b: &[u8], start: usize, end: usize) -> Vec<El> {
        let mut out = Vec::new();
        let mut at = start;
        while at < end {
            let Ok((id, il)) = id(b, at) else { break };
            let Ok((sz, sl)) = size(b, at + il) else {
                break;
            };
            let s = at + il + sl;
            let e = match sz {
                Some(sz) => s.saturating_add(usize::try_from(sz).unwrap_or(usize::MAX)),
                None => end,
            };
            out.push(El {
                id,
                start: s,
                end: e.min(end),
            });
            if e >= end {
                break;
            }
            at = e;
        }
        out
    }

    fn uint(b: &[u8], el: &El) -> Option<u64> {
        let s = b.get(el.start..el.end)?;
        (s.len() <= 8).then(|| s.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
    }

    fn float(b: &[u8], el: &El) -> Option<f64> {
        let s = b.get(el.start..el.end)?;
        match s.len() {
            4 => Some(f64::from(f32::from_be_bytes([s[0], s[1], s[2], s[3]]))),
            8 => {
                let mut a = [0u8; 8];
                a.copy_from_slice(s);
                Some(f64::from_be_bytes(a))
            }
            _ => None,
        }
    }

    fn text(b: &[u8], el: &El) -> String {
        let s = b.get(el.start..el.end).unwrap_or(&[]);
        let s = s.split(|c| *c == 0).next().unwrap_or(&[]);
        String::from_utf8_lossy(s).into_owned()
    }

    /// `whole`: `b` is the whole file (a missing Duration is then read from the last block,
    /// as in files from `MediaRecorder`).
    pub(super) fn probe(b: &[u8], whole: bool, info: &mut MediaInfo) -> R<()> {
        let top = elements(b, 0, b.len());
        if top.first().map(|e| e.id) != Some(EBML) {
            return Err(Bad);
        }
        let seg = top.iter().find(|e| e.id == SEGMENT).ok_or(Bad)?;
        let mut scale = 1_000_000u64;
        let mut duration: Option<f64> = None;
        let mut found_tracks = false;
        let mut last_block = None;
        for el in elements(b, seg.start, seg.end) {
            match el.id {
                INFO => {
                    for i in elements(b, el.start, el.end) {
                        match i.id {
                            TIMECODE_SCALE => {
                                scale = uint(b, &i).filter(|v| *v > 0).unwrap_or(scale)
                            }
                            DURATION => duration = float(b, &i),
                            _ => {}
                        }
                    }
                }
                TRACKS => {
                    found_tracks = true;
                    for t in elements(b, el.start, el.end)
                        .iter()
                        .filter(|t| t.id == TRACK_ENTRY)
                    {
                        track(b, t, info);
                    }
                }
                CLUSTER => {
                    if duration.is_none() && whole {
                        last_block = last_block_time(b, el.start, seg.end);
                    }
                    break;
                }
                _ => {}
            }
        }
        if !found_tracks && duration.is_none() {
            return Err(Bad);
        }
        info.duration_ms = duration
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(|d| (d * scale as f64 / 1_000_000.0).round() as u64);
        if info.duration_ms.is_none() {
            // The last block's time plus one frame.
            let frame_ms = info.fps.map_or(0.0, |f| 1000.0 / f);
            info.duration_ms = last_block
                .map(|t| (t as f64 * scale as f64 / 1_000_000.0 + frame_ms).round() as u64)
                .filter(|d| *d > 0);
        }
        Ok(())
    }

    fn track(b: &[u8], t: &El, info: &mut MediaInfo) {
        let mut kind = 0u64;
        let mut codec = String::new();
        let mut frame_ns = None;
        let mut w = None;
        let mut h = None;
        let mut alpha = false;
        for e in elements(b, t.start, t.end) {
            match e.id {
                TRACK_TYPE => kind = uint(b, &e).unwrap_or(0),
                CODEC_ID => codec = text(b, &e),
                DEFAULT_DURATION => frame_ns = uint(b, &e).filter(|v| *v > 0),
                VIDEO => {
                    for v in elements(b, e.start, e.end) {
                        match v.id {
                            PIXEL_WIDTH => w = uint(b, &v).and_then(|x| u32::try_from(x).ok()),
                            PIXEL_HEIGHT => h = uint(b, &v).and_then(|x| u32::try_from(x).ok()),
                            ALPHA_MODE => alpha = uint(b, &v) == Some(1),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        match kind {
            1 if info.video_codec.is_none() => {
                info.video_codec = Some(
                    match codec.as_str() {
                        "V_VP8" => "vp8",
                        "V_VP9" => "vp9",
                        "V_AV1" => "av1",
                        "V_MPEG4/ISO/AVC" => "h264",
                        "V_MPEGH/ISO/HEVC" => "hevc",
                        other => return set_other(info, other, w, h, alpha, frame_ns),
                    }
                    .to_string(),
                );
                info.width = w;
                info.height = h;
                info.alpha = alpha;
                info.fps = frame_ns.and_then(|ns| fps(1e9 / ns as f64));
            }
            2 if info.audio_codec.is_none() => {
                let c = match codec.as_str() {
                    "A_OPUS" => "opus".to_string(),
                    "A_VORBIS" => "vorbis".to_string(),
                    "A_AAC" => "aac".to_string(),
                    "A_MPEG/L3" => "mp3".to_string(),
                    "A_FLAC" => "flac".to_string(),
                    c if c.starts_with("A_PCM") => "pcm".to_string(),
                    c if c.starts_with("A_AAC") => "aac".to_string(),
                    c => c.trim_start_matches("A_").to_ascii_lowercase(),
                };
                info.audio_codec = Some(c);
            }
            _ => {}
        }
    }

    fn set_other(
        info: &mut MediaInfo,
        codec: &str,
        w: Option<u32>,
        h: Option<u32>,
        alpha: bool,
        frame_ns: Option<u64>,
    ) {
        info.video_codec = Some(codec.trim_start_matches("V_").to_ascii_lowercase());
        info.width = w;
        info.height = h;
        info.alpha = alpha;
        info.fps = frame_ns.and_then(|ns| fps(1e9 / ns as f64));
    }
}

// ---- Audio (MP3, WAV, AAC ADTS) ------------------------------------------------------------------

mod audio {
    use super::*;

    /// Bytes of an ID3v2 tag at the start (0 without one).
    fn id3_len(b: &[u8]) -> usize {
        if b.len() < 10 || &b[..3] != b"ID3" {
            return 0;
        }
        let sz = b[6..10]
            .iter()
            .fold(0usize, |a, &x| (a << 7) | usize::from(x & 0x7F));
        let footer = if b[5] & 0x10 != 0 { 10 } else { 0 };
        10 + sz + footer
    }

    struct Frame {
        /// 1 = MPEG-1, 2 = MPEG-2, 25 = MPEG-2.5.
        version: u8,
        layer: u8,
        bitrate: u32,
        rate: u32,
        mono: bool,
        len: usize,
    }

    fn mp3_frame(b: &[u8], at: usize) -> Option<Frame> {
        let h = b.get(at..at + 4)?;
        if h[0] != 0xFF || h[1] & 0xE0 != 0xE0 {
            return None;
        }
        let version = match (h[1] >> 3) & 3 {
            0 => 25,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        let layer = match (h[1] >> 1) & 3 {
            1 => 3,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        let bi = usize::from(h[2] >> 4);
        let si = usize::from((h[2] >> 2) & 3);
        if bi == 0 || bi == 15 || si == 3 {
            return None;
        }
        const V1L1: [u32; 15] = [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
        ];
        const V1L2: [u32; 15] = [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
        ];
        const V1L3: [u32; 15] = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ];
        const V2L1: [u32; 15] = [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ];
        const V2L23: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
        let kbps = match (version, layer) {
            (1, 1) => V1L1[bi],
            (1, 2) => V1L2[bi],
            (1, _) => V1L3[bi],
            (_, 1) => V2L1[bi],
            _ => V2L23[bi],
        };
        let rate = [44100, 48000, 32000][si]
            / match version {
                1 => 1,
                2 => 2,
                _ => 4,
            };
        let pad = usize::from((h[2] >> 1) & 1);
        let bitrate = kbps * 1000;
        let len = if layer == 1 {
            (12 * bitrate as usize / rate as usize + pad) * 4
        } else {
            let k = if layer == 3 && version != 1 { 72 } else { 144 };
            k * bitrate as usize / rate as usize + pad
        };
        Some(Frame {
            version,
            layer,
            bitrate,
            rate,
            mono: (h[3] >> 6) == 3,
            len,
        })
    }

    fn samples_per_frame(f: &Frame) -> u64 {
        match (f.layer, f.version) {
            (1, _) => 384,
            (2, _) => 1152,
            (_, 1) => 1152,
            _ => 576,
        }
    }

    pub(super) fn mp3(b: &[u8], len: u64, info: &mut MediaInfo) -> R<()> {
        let start = id3_len(b);
        // The first frame header followed by a second one where its length says (a false sync
        // inside junk is skipped).
        let mut at = start;
        let frame = loop {
            if at + 4 > b.len() || at > start + 64 * 1024 {
                return Err(Bad);
            }
            if let Some(f) = mp3_frame(b, at) {
                let next = at + f.len;
                if f.len > 4 && (next + 4 > b.len() || mp3_frame(b, next).is_some()) {
                    break f;
                }
            }
            at += 1;
        };
        info.audio_codec = Some("mp3".into());
        let side = match (frame.version, frame.mono) {
            (1, false) => 32,
            (1, true) => 17,
            (_, false) => 17,
            (_, true) => 9,
        };
        let spf = samples_per_frame(&frame);
        let xing = at + 4 + side;
        let mut frames = None;
        if matches!(b.get(xing..xing + 4), Some(b"Xing") | Some(b"Info")) {
            let flags = be32(b, xing + 4)?;
            if flags & 1 != 0 {
                frames = Some(u64::from(be32(b, xing + 8)?));
            }
        } else if b.get(at + 36..at + 40) == Some(b"VBRI") {
            frames = Some(u64::from(be32(b, at + 50)?));
        }
        info.duration_ms = match frames {
            Some(n) if n > 0 => ms(n * spf, u64::from(frame.rate)),
            _ => {
                let audio = len.saturating_sub(at as u64);
                ms(audio * 8, u64::from(frame.bitrate))
            }
        };
        Ok(())
    }

    pub(super) fn wav<F: Read + Seek>(
        f: &mut F,
        len: u64,
        b: &[u8],
        info: &mut MediaInfo,
    ) -> R<()> {
        if b.get(..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WAVE") {
            return Err(Bad);
        }
        let mut at = 12u64;
        let mut byte_rate = 0u64;
        let mut format = None;
        let mut data = None;
        let mut hdr = [0u8; 24];
        while at + 8 <= len {
            f.seek(SeekFrom::Start(at))?;
            let n = usize::try_from((len - at).min(24)).map_err(|_| Bad)?;
            f.read_exact(&mut hdr[..n])?;
            let size = u64::from(le32(&hdr, 4)?);
            match &hdr[..4] {
                b"fmt " => {
                    format = Some(le16(&hdr, 8)?);
                    byte_rate = u64::from(le32(&hdr, 16)?);
                }
                b"data" => {
                    let avail = len - at - 8;
                    data = Some(if size == 0 || size == u64::from(u32::MAX) {
                        avail
                    } else {
                        size.min(avail)
                    });
                    break;
                }
                _ => {}
            }
            at += 8 + size + (size & 1);
        }
        let format = format.ok_or(Bad)?;
        info.audio_codec = Some(match format {
            1 | 3 | 0xFFFE => "pcm".to_string(),
            0x55 => "mp3".to_string(),
            6 => "alaw".to_string(),
            7 => "mulaw".to_string(),
            n => format!("wav-{n}"),
        });
        info.duration_ms = data.and_then(|d| ms(d, byte_rate));
        Ok(())
    }

    /// Frames walked inside the head before the duration is estimated.
    const ADTS_WALK_MAX: usize = 4000;

    pub(super) fn adts(b: &[u8], len: u64, info: &mut MediaInfo) -> R<()> {
        const RATES: [u32; 13] = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ];
        let start = id3_len(b);
        let mut at = start;
        let mut frames = 0u64;
        let mut blocks = 0u64;
        let mut rate = 0u32;
        while frames < ADTS_WALK_MAX as u64 {
            let Some(h) = b.get(at..at + 7) else { break };
            if h[0] != 0xFF || h[1] & 0xF6 != 0xF0 {
                if frames == 0 {
                    return Err(Bad);
                }
                break;
            }
            let si = usize::from((h[2] >> 2) & 0xF);
            rate = *RATES.get(si).ok_or(Bad)?;
            let flen =
                (usize::from(h[3] & 3) << 11) | (usize::from(h[4]) << 3) | usize::from(h[5] >> 5);
            if flen < 7 {
                return Err(Bad);
            }
            blocks += u64::from(h[6] & 3) + 1;
            frames += 1;
            at += flen;
        }
        if frames == 0 || rate == 0 {
            return Err(Bad);
        }
        info.audio_codec = Some("aac".into());
        let walked = (at - start) as u64;
        let total_blocks = if (at as u64) >= len || walked == 0 {
            blocks
        } else {
            // Beyond the walk: the same average frame size.
            blocks * (len - start as u64) / walked
        };
        info.duration_ms = ms(total_blocks * 1024, u64::from(rate));
        Ok(())
    }
}

// ---- Images ----------------------------------------------------------------------------------

mod image {
    use super::*;

    pub(super) fn probe(b: &[u8], info: &mut MediaInfo) -> R<()> {
        let (w, h) = match info.container.as_str() {
            "png" => {
                if b.get(..8) != Some(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
                    return Err(Bad);
                }
                (be32(b, 16)?, be32(b, 20)?)
            }
            "gif" => {
                if !matches!(b.get(..6), Some(b"GIF87a") | Some(b"GIF89a")) {
                    return Err(Bad);
                }
                (u32::from(le16(b, 6)?), u32::from(le16(b, 8)?))
            }
            "webp" => webp(b)?,
            "jpg" => jpeg(b)?,
            _ => return Err(Bad),
        };
        if w > 0 && h > 0 {
            info.width = Some(w);
            info.height = Some(h);
        }
        Ok(())
    }

    fn webp(b: &[u8]) -> R<(u32, u32)> {
        if b.get(..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WEBP") {
            return Err(Bad);
        }
        match b.get(12..16).ok_or(Bad)? {
            b"VP8 " => Ok((
                u32::from(le16(b, 26)? & 0x3FFF),
                u32::from(le16(b, 28)? & 0x3FFF),
            )),
            b"VP8L" => {
                let v = le32(b, 21)?;
                Ok(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => {
                let w = u32::from_le_bytes([*b.get(24).ok_or(Bad)?, b[25], b[26], 0]) + 1;
                let h = u32::from_le_bytes([*b.get(27).ok_or(Bad)?, b[28], b[29], 0]) + 1;
                Ok((w, h))
            }
            _ => Err(Bad),
        }
    }

    fn jpeg(b: &[u8]) -> R<(u32, u32)> {
        if b.get(..3) != Some(&[0xFF, 0xD8, 0xFF]) {
            return Err(Bad);
        }
        let mut at = 2;
        loop {
            while b.get(at).copied() == Some(0xFF) && b.get(at + 1).copied() == Some(0xFF) {
                at += 1;
            }
            if b.get(at).copied() != Some(0xFF) {
                return Err(Bad);
            }
            let m = *b.get(at + 1).ok_or(Bad)?;
            if (0xD0..=0xD9).contains(&m) || m == 0x01 {
                at += 2;
                continue;
            }
            let seg = usize::from(be16(b, at + 2)?);
            if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                let h = u32::from(be16(b, at + 5)?);
                let w = u32::from(be16(b, at + 7)?);
                return Ok((w, h));
            }
            if seg < 2 {
                return Err(Bad);
            }
            at += 2 + seg;
        }
    }
}

// ---- Cache -------------------------------------------------------------------------------------

struct Entry {
    size: u64,
    mtime: Option<SystemTime>,
    info: MediaInfo,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Entry>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Entry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The probe of `<dir>/<name>`, from the cache while the file's size and modification time are
/// unchanged (one `stat`); `None` when the file doesn't exist. The cache lock is never held
/// while the file is read.
pub fn info_cached(dir: &Path, name: &str) -> Option<MediaInfo> {
    let path = dir.join(name);
    let meta = fs::metadata(&path).ok().filter(|m| m.is_file())?;
    let size = meta.len();
    let mtime = meta.modified().ok();
    {
        let c = cache().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(e) = c.get(&path) {
            if e.size == size && e.mtime == mtime {
                return Some(e.info.clone());
            }
        }
    }
    let info = probe_file(&path, name);
    let mut c = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if c.len() >= CACHE_MAX {
        c.clear();
    }
    c.insert(
        path,
        Entry {
            size,
            mtime,
            info: info.clone(),
        },
    );
    Some(info)
}

#[cfg(test)]
mod tests;
