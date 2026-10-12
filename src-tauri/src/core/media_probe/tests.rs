//! Probe tests on byte layouts built here (no binary fixtures needed).

use super::*;
use crate::core::model::MediaIssueCode;

fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(payload);
    v
}

fn full(version: u8, rest: &[u8]) -> Vec<u8> {
    let mut v = vec![version, 0, 0, 0];
    v.extend_from_slice(rest);
    v
}

fn mvhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut p = vec![0u8; 8];
    p.extend_from_slice(&timescale.to_be_bytes());
    p.extend_from_slice(&duration.to_be_bytes());
    p.extend_from_slice(&[0u8; 80]);
    bx(b"mvhd", &full(0, &p))
}

fn tkhd(w: u32, h: u32) -> Vec<u8> {
    let mut p = vec![0u8; 72];
    p.extend_from_slice(&(w << 16).to_be_bytes());
    p.extend_from_slice(&(h << 16).to_be_bytes());
    bx(b"tkhd", &full(0, &p))
}

fn mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut p = vec![0u8; 8];
    p.extend_from_slice(&timescale.to_be_bytes());
    p.extend_from_slice(&duration.to_be_bytes());
    p.extend_from_slice(&[0u8; 4]);
    bx(b"mdhd", &full(0, &p))
}

fn hdlr(kind: &[u8; 4]) -> Vec<u8> {
    let mut p = vec![0u8; 4];
    p.extend_from_slice(kind);
    p.extend_from_slice(&[0u8; 13]);
    bx(b"hdlr", &full(0, &p))
}

fn video_entry(
    fourcc: &[u8; 4],
    w: u16,
    h: u16,
    compressor: &str,
    depth: u16,
    kids: &[u8],
) -> Vec<u8> {
    let mut p = vec![0u8; 8];
    p.extend_from_slice(&[0u8; 16]);
    p.extend_from_slice(&w.to_be_bytes());
    p.extend_from_slice(&h.to_be_bytes());
    p.extend_from_slice(&[0u8; 14]);
    let mut cn = [0u8; 32];
    cn[0] = compressor.len() as u8;
    cn[1..=compressor.len()].copy_from_slice(compressor.as_bytes());
    p.extend_from_slice(&cn);
    p.extend_from_slice(&depth.to_be_bytes());
    p.extend_from_slice(&[0xFF, 0xFF]);
    p.extend_from_slice(kids);
    bx(fourcc, &p)
}

fn audio_entry(fourcc: &[u8; 4]) -> Vec<u8> {
    bx(fourcc, &[0u8; 28])
}

fn stbl(entry: &[u8], stts: &[(u32, u32)]) -> Vec<u8> {
    let mut sd = full(0, &1u32.to_be_bytes());
    sd.extend_from_slice(entry);
    let mut st = full(0, &(stts.len() as u32).to_be_bytes());
    for (c, d) in stts {
        st.extend_from_slice(&c.to_be_bytes());
        st.extend_from_slice(&d.to_be_bytes());
    }
    let mut p = bx(b"stsd", &sd);
    p.extend(bx(b"stts", &st));
    bx(b"stbl", &p)
}

fn trak(
    handler: &[u8; 4],
    size: (u32, u32),
    ts: u32,
    dur: u32,
    entry: &[u8],
    stts: &[(u32, u32)],
) -> Vec<u8> {
    let minf = bx(b"minf", &stbl(entry, stts));
    let mut mdia = mdhd(ts, dur);
    mdia.extend(hdlr(handler));
    mdia.extend(minf);
    let mut t = tkhd(size.0, size.1);
    t.extend(bx(b"mdia", &mdia));
    bx(b"trak", &t)
}

fn avcc(profile: u8, level: u8) -> Vec<u8> {
    bx(b"avcC", &[1, profile, 0, level, 0xFF, 0xE0])
}

/// An MP4: ftyp, then moov and mdat in the given order.
fn mp4(brand: &[u8; 4], moov_first: bool, tracks: &[Vec<u8>]) -> Vec<u8> {
    let mut f = bx(
        b"ftyp",
        &[brand.as_slice(), &[0, 0, 0, 0], b"isom"].concat(),
    );
    let mut moov = mvhd(1000, 12_500);
    for t in tracks {
        moov.extend_from_slice(t);
    }
    let moov = bx(b"moov", &moov);
    let mdat = bx(b"mdat", &[0u8; 64]);
    if moov_first {
        f.extend(moov);
        f.extend(mdat);
    } else {
        f.extend(mdat);
        f.extend(moov);
    }
    f
}

fn h264_track() -> Vec<u8> {
    trak(
        b"vide",
        (1920, 1080),
        25_000,
        312_500,
        &video_entry(b"avc1", 1920, 1080, "", 24, &avcc(100, 41)),
        &[(312, 1000), (1, 500)],
    )
}

#[test]
fn mp4_h264_aac_fast_start() {
    let aac = trak(
        b"soun",
        (0, 0),
        48_000,
        600_000,
        &audio_entry(b"mp4a"),
        &[(586, 1024)],
    );
    let f = mp4(b"isom", true, &[h264_track(), aac]);
    let i = probe_bytes(&f, "clip.mp4");
    assert_eq!(i.kind, MediaKind::Video);
    assert_eq!(i.container, "mp4");
    assert_eq!(i.duration_ms, Some(12_500));
    assert_eq!((i.width, i.height), (Some(1920), Some(1080)));
    assert_eq!(i.video_codec.as_deref(), Some("h264 High 4.1"));
    assert_eq!(i.audio_codec.as_deref(), Some("aac"));
    assert_eq!(i.fps, Some(25.04)); // 313 samples over 312500 units at 25000 per second
    assert_eq!(i.fast_start, Some(true));
    assert!(!i.alpha);
    assert!(i.issues.is_empty(), "{:?}", i.issues);
}

#[test]
fn mp4_fps_from_stts() {
    let t = trak(
        b"vide",
        (1280, 720),
        30_000,
        300_300,
        &video_entry(b"avc1", 1280, 720, "", 24, &avcc(77, 31)),
        &[(300, 1001)],
    );
    let i = probe_bytes(&mp4(b"isom", true, &[t]), "a.mp4");
    assert_eq!(i.fps, Some(29.97));
    assert_eq!(i.video_codec.as_deref(), Some("h264 Main 3.1"));
}

#[test]
fn mp4_not_fast_start_warns() {
    let f = mp4(b"isom", false, &[h264_track()]);
    let i = probe_bytes(&f, "slow.mp4");
    assert_eq!(i.fast_start, Some(false));
    assert_eq!(i.issues.len(), 1);
    assert_eq!(i.issues[0].code, MediaIssueCode::SlowStart);
}

#[test]
fn hevc_and_hevc_alpha_and_prores() {
    let hevc = trak(
        b"vide",
        (3840, 2160),
        600,
        6000,
        &video_entry(b"hvc1", 3840, 2160, "HEVC", 24, &[]),
        &[(250, 24)],
    );
    let i = probe_bytes(&mp4(b"isom", true, &[hevc]), "uhd.mp4");
    assert_eq!(i.video_codec.as_deref(), Some("hevc"));
    assert!(!i.alpha);
    assert!(i
        .issues
        .iter()
        .any(|x| x.code == MediaIssueCode::NeedsExtension));
    let alpha = trak(
        b"vide",
        (1920, 1080),
        600,
        6000,
        &video_entry(b"hvc1", 1920, 1080, "HEVC with Alpha", 32, &[]),
        &[(250, 24)],
    );
    let i = probe_bytes(&mp4(b"qt  ", true, &[alpha]), "key.mov");
    assert_eq!(i.container, "mov");
    assert!(i.alpha);
    assert_eq!(i.issues[0].code, MediaIssueCode::Unsupported);
    let prores = trak(
        b"vide",
        (1920, 1080),
        600,
        6000,
        &video_entry(b"ap4h", 1920, 1080, "Apple ProRes 4444", 32, &[]),
        &[(250, 24)],
    );
    let i = probe_bytes(&mp4(b"qt  ", true, &[prores]), "pr.mov");
    assert_eq!(i.video_codec.as_deref(), Some("prores"));
    assert!(i.alpha);
}

#[test]
fn av1_vp9_mp4_and_mpeg4() {
    for (cc, name) in [(b"av01", "av1"), (b"vp09", "vp9"), (b"mp4v", "mpeg4")] {
        let t = trak(
            b"vide",
            (640, 360),
            1000,
            1000,
            &video_entry(cc, 640, 360, "", 24, &[]),
            &[(25, 40)],
        );
        let i = probe_bytes(&mp4(b"isom", true, &[t]), "x.mp4");
        assert_eq!(i.video_codec.as_deref(), Some(name));
        assert_eq!(i.fps, Some(25.0));
    }
}

#[test]
fn m4a_is_audio() {
    let aac = trak(
        b"soun",
        (0, 0),
        44_100,
        441_000,
        &audio_entry(b"mp4a"),
        &[(430, 1024)],
    );
    let i = probe_bytes(&mp4(b"M4A ", true, &[aac]), "song.m4a");
    assert_eq!(i.kind, MediaKind::Audio);
    assert_eq!(i.container, "mp4");
    assert_eq!(i.audio_codec.as_deref(), Some("aac"));
    assert_eq!(i.duration_ms, Some(12_500));
}

#[test]
fn moov_far_into_the_file_is_found_by_seeking() {
    // mdat larger than the 1 MB head: the moov lies beyond it.
    let mut f = bx(b"ftyp", b"isom\0\0\0\0isom");
    let big = vec![0u8; HEAD_MAX + 4096];
    f.extend(bx(b"mdat", &big));
    let mut moov = mvhd(1000, 4000);
    moov.extend(h264_track());
    f.extend(bx(b"moov", &moov));
    let i = probe_bytes(&f, "big.mp4");
    assert_eq!(i.duration_ms, Some(4000));
    assert_eq!(i.fast_start, Some(false));
    assert_eq!(i.video_codec.as_deref(), Some("h264 High 4.1"));
}

// ---- EBML ------------------------------------------------------------------------------------

fn el(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut v: Vec<u8> = id
        .to_be_bytes()
        .iter()
        .copied()
        .skip_while(|b| *b == 0)
        .collect();
    let n = payload.len() as u64;
    v.extend_from_slice(&(0x0100_0000_0000_0000u64 | n).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

fn uint_el(id: u32, v: u64) -> Vec<u8> {
    el(id, &v.to_be_bytes())
}

fn webm(codec: &str, alpha: bool, unknown_segment_size: bool) -> Vec<u8> {
    let header = el(0x1A45_DFA3, &el(0x4282, b"webm"));
    let mut info = uint_el(0x2A_D7B1, 1_000_000);
    info.extend(el(0x4489, &8_000.5f64.to_be_bytes()));
    let mut video = uint_el(0xB0, 1280);
    video.extend(uint_el(0xBA, 720));
    if alpha {
        video.extend(uint_el(0x53C0, 1));
    }
    let mut te = uint_el(0x83, 1);
    te.extend(el(0x86, codec.as_bytes()));
    te.extend(uint_el(0x23_E383, 40_000_000));
    te.extend(el(0xE0, &video));
    let mut ta = uint_el(0x83, 2);
    ta.extend(el(0x86, b"A_OPUS"));
    let mut tracks = el(0xAE, &te);
    tracks.extend(el(0xAE, &ta));
    let mut seg = el(0x1549_A966, &info);
    seg.extend(el(0x1654_AE6B, &tracks));
    seg.extend(el(0x1F43_B675, &[0u8; 32]));
    let mut f = header;
    if unknown_segment_size {
        f.extend_from_slice(&[
            0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]);
        f.extend(seg);
    } else {
        f.extend(el(0x1853_8067, &seg));
    }
    f
}

#[test]
fn webm_vp9_alpha_and_vp8() {
    let i = probe_bytes(&webm("V_VP9", true, false), "logo.webm");
    assert_eq!(i.container, "webm");
    assert_eq!(i.video_codec.as_deref(), Some("vp9"));
    assert_eq!(i.audio_codec.as_deref(), Some("opus"));
    assert!(i.alpha);
    assert_eq!(i.duration_ms, Some(8001));
    assert_eq!((i.width, i.height), (Some(1280), Some(720)));
    assert_eq!(i.fps, Some(25.0));
    assert_eq!(i.fast_start, None);
    assert_eq!(i.issues[0].code, MediaIssueCode::NoAlpha);
    let i = probe_bytes(&webm("V_VP8", false, true), "a.webm");
    assert_eq!(i.video_codec.as_deref(), Some("vp8"));
    assert!(!i.alpha);
    let i = probe_bytes(&webm("V_AV1", false, false), "a.webm");
    assert_eq!(i.video_codec.as_deref(), Some("av1"));
}

// ---- Audio -----------------------------------------------------------------------------------

fn mp3_frames(n: usize, xing: Option<u32>) -> Vec<u8> {
    // MPEG-1 Layer III, 128 kbps, 44.1 kHz, stereo: 417 bytes per frame.
    let hdr = [0xFF, 0xFB, 0x90, 0x00];
    let mut f = b"ID3\x03\0\0\0\0\0\x0A".to_vec();
    f.extend_from_slice(&[0u8; 10]);
    for i in 0..n {
        let mut fr = vec![0u8; 417];
        fr[..4].copy_from_slice(&hdr);
        if let (0, Some(frames)) = (i, xing) {
            fr[36..40].copy_from_slice(b"Xing");
            fr[40..44].copy_from_slice(&1u32.to_be_bytes());
            fr[44..48].copy_from_slice(&frames.to_be_bytes());
        }
        f.extend(fr);
    }
    f
}

#[test]
fn mp3_xing_and_cbr() {
    let i = probe_bytes(&mp3_frames(40, Some(1000)), "a.mp3");
    assert_eq!(i.kind, MediaKind::Audio);
    assert_eq!(i.audio_codec.as_deref(), Some("mp3"));
    assert_eq!(i.duration_ms, Some(26_122)); // 1000 × 1152 / 44100
                                             // CBR estimate: 100 frames of 417 bytes at 128 kbps ≈ 2.6 s.
    let i = probe_bytes(&mp3_frames(100, None), "b.mp3");
    let d = i.duration_ms.unwrap();
    assert!((2600..2620).contains(&d), "{d}");
    assert!(i.issues.is_empty());
}

#[test]
fn wav_duration() {
    let mut f = b"RIFF\0\0\0\0WAVE".to_vec();
    let mut fmt = vec![];
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&48_000u32.to_le_bytes());
    fmt.extend_from_slice(&192_000u32.to_le_bytes());
    fmt.extend_from_slice(&4u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(b"fmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend(fmt);
    f.extend_from_slice(b"LIST");
    f.extend_from_slice(&3u32.to_le_bytes());
    f.extend_from_slice(&[0, 0, 0, 0]); // 3 bytes + pad
    f.extend_from_slice(b"data");
    f.extend_from_slice(&96_000u32.to_le_bytes());
    f.extend(vec![0u8; 96_000]);
    let i = probe_bytes(&f, "tone.wav");
    assert_eq!(i.audio_codec.as_deref(), Some("pcm"));
    assert_eq!(i.duration_ms, Some(500));
}

fn adts(frames: usize) -> Vec<u8> {
    let mut f = Vec::new();
    for _ in 0..frames {
        let len = 200usize;
        // AAC LC, 48 kHz (index 3), stereo, no CRC.
        let h = [
            0xFF,
            0xF1,
            (1 << 6) | (3 << 2),
            (2 << 6) | ((len >> 11) as u8 & 3),
            ((len >> 3) & 0xFF) as u8,
            (((len & 7) << 5) as u8) | 0x1F,
            0xFC,
        ];
        f.extend_from_slice(&h);
        f.extend(vec![0u8; len - 7]);
    }
    f
}

#[test]
fn adts_walk_and_estimate() {
    let i = probe_bytes(&adts(375), "a.aac");
    assert_eq!(i.audio_codec.as_deref(), Some("aac"));
    assert_eq!(i.duration_ms, Some(8000)); // 375 × 1024 / 48000
                                           // Longer than the head: estimated from the walked part.
    let i = probe_bytes(&adts(10_000), "long.aac");
    let d = i.duration_ms.unwrap();
    assert!((213_000..=213_400).contains(&d), "{d}");
}

// ---- Images and errors -------------------------------------------------------------------------

#[test]
fn image_sizes() {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&640u32.to_be_bytes());
    png.extend_from_slice(&480u32.to_be_bytes());
    let i = probe_bytes(&png, "a.png");
    assert_eq!(
        (i.kind, i.width, i.height),
        (MediaKind::Image, Some(640), Some(480))
    );
    let gif = [b"GIF89a".as_slice(), &[10, 0, 20, 0]].concat();
    assert_eq!(probe_bytes(&gif, "a.gif").height, Some(20));
    let jpg = [
        0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0x02, 0xD0, 0x05, 0x00, 3,
    ];
    let i = probe_bytes(&jpg, "a.jpg");
    assert_eq!((i.width, i.height), (Some(1280), Some(720)));
    let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
    webp.extend_from_slice(&[0u8; 8]);
    webp.extend_from_slice(&[0x7F, 0x07, 0, 0x37, 0x04, 0]);
    let i = probe_bytes(&webp, "a.webp");
    assert_eq!((i.width, i.height), (Some(1920), Some(1080)));
}

#[test]
fn truncated_and_corrupt_files_are_unreadable_never_panic() {
    let good = mp4(b"isom", true, &[h264_track()]);
    for cut in [0, 4, 9, 20, 40, good.len() / 2, good.len() - 3] {
        let i = probe_bytes(&good[..cut], "t.mp4");
        assert!(
            i.issues
                .iter()
                .any(|x| x.code == MediaIssueCode::Unreadable)
                || i.duration_ms.is_some(),
            "cut {cut}"
        );
    }
    let i = probe_bytes(b"not a video at all", "x.mp4");
    assert_eq!(i.issues[0].code, MediaIssueCode::Unreadable);
    assert_eq!(
        probe_bytes(&[], "x.webm").issues[0].code,
        MediaIssueCode::Unreadable
    );
    assert_eq!(
        probe_bytes(b"\xFF\xFB", "x.mp3").issues[0].code,
        MediaIssueCode::Unreadable
    );
    assert_eq!(
        probe_bytes(b"RIFF", "x.wav").issues[0].code,
        MediaIssueCode::Unreadable
    );
    // Every prefix of every format, and garbage, must not panic.
    let mut samples = vec![
        good,
        webm("V_VP9", true, false),
        mp3_frames(3, Some(3)),
        adts(3),
    ];
    let mut x = 12345u32;
    samples.push(
        (0..4096)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect(),
    );
    for s in &samples {
        for n in [
            "a.mp4", "a.webm", "a.mp3", "a.wav", "a.aac", "a.jpg", "a.webp",
        ] {
            for cut in (0..s.len()).step_by(7) {
                let _ = probe_bytes(&s[..cut], n);
            }
        }
    }
    // Box sizes pointing past the end.
    let mut evil = bx(b"ftyp", b"isom");
    evil.extend_from_slice(&u32::MAX.to_be_bytes());
    evil.extend_from_slice(b"moov");
    let _ = probe_bytes(&evil, "e.mp4");
}

#[test]
fn cache_hits_by_size_and_mtime() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("c.mp4"),
        mp4(b"isom", true, &[h264_track()]),
    )
    .unwrap();
    let a = info_cached(dir.path(), "c.mp4").unwrap();
    assert_eq!(a.duration_ms, Some(12_500));
    let b = info_cached(dir.path(), "c.mp4").unwrap();
    assert_eq!(a, b);
    assert!(info_cached(dir.path(), "missing.mp4").is_none());
}

/// A virtual file: `head`, then zeros up to `len - tail.len()`, then `tail`. Counts bytes read.
struct Virtual {
    head: Vec<u8>,
    tail: Vec<u8>,
    len: u64,
    pos: u64,
    read: u64,
}

impl Read for Virtual {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = (buf.len() as u64).min(self.len.saturating_sub(self.pos)) as usize;
        let tail_at = self.len - self.tail.len() as u64;
        for (i, b) in buf[..n].iter_mut().enumerate() {
            let p = self.pos + i as u64;
            *b = if p < self.head.len() as u64 {
                self.head[p as usize]
            } else if p >= tail_at {
                self.tail[(p - tail_at) as usize]
            } else {
                0
            };
        }
        self.pos += n as u64;
        self.read += n as u64;
        Ok(n)
    }
}

impl Seek for Virtual {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::End(d) => (self.len as i64 + d) as u64,
            SeekFrom::Current(d) => (self.pos as i64 + d) as u64,
        };
        Ok(self.pos)
    }
}

#[test]
fn probing_a_4_gb_mp4_reads_headers_only() {
    // 4 GB of mdat, then the moov (a virtual file: nothing on disk).
    let mut head = bx(b"ftyp", b"isom\0\0\0\0isom");
    let big: u64 = 4 * 1024 * 1024 * 1024;
    head.extend_from_slice(&1u32.to_be_bytes());
    head.extend_from_slice(b"mdat");
    head.extend_from_slice(&(big + 16).to_be_bytes());
    let mut moov = mvhd(1000, 3_600_000);
    moov.extend(h264_track());
    let tail = bx(b"moov", &moov);
    let len = head.len() as u64 + big + tail.len() as u64;
    let mut f = Virtual {
        head,
        tail,
        len,
        pos: 0,
        read: 0,
    };
    let t = std::time::Instant::now();
    let i = probe_reader(&mut f, len, "mp4").ok().unwrap();
    let took = t.elapsed();
    assert_eq!(i.duration_ms, Some(3_600_000));
    assert_eq!(i.fast_start, Some(false));
    assert!(f.read < 2 * HEAD_MAX as u64, "read {} bytes", f.read);
    assert!(took.as_millis() < 100, "{took:?}");
    eprintln!("probe of a 4 GB MP4: {took:?}, {} bytes read", f.read);
}

/// The e2e clips (recorded in Edge by `e2e/media/make-fixtures.mjs`): MediaRecorder WebM has no
/// Duration (read from the last block); the MP4 is fragmented (duration from the `trun`s).
#[test]
fn recorded_e2e_clips() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../e2e/media");
    for (name, codec, alpha) in [
        ("clip-vp8.webm", "vp8", false),
        ("clip-vp9.webm", "vp9", false),
        ("clip-vp9-alpha.webm", "vp9", true),
        ("clip-h264.mp4", "h264", false),
    ] {
        let path = dir.join(name);
        if !path.exists() {
            eprintln!("{name} missing; skipped");
            continue;
        }
        let i = probe_file(&path, name);
        eprintln!("{name}: {i:?}");
        assert!(
            i.video_codec
                .as_deref()
                .is_some_and(|c| c.starts_with(codec)),
            "{name}: {:?}",
            i.video_codec
        );
        // Chromium's MediaRecorder writes AlphaMode 1 on every VP8/VP9 track (opaque canvases
        // too), so the header says alpha for all three WebM clips.
        assert!(!alpha || i.alpha, "{name}");
        assert!(i.width.is_some() && i.height.is_some(), "{name}");
        let d = i.duration_ms.expect("duration");
        assert!((500..=60_000).contains(&d), "{name}: {d}");
        assert!(
            !i.issues
                .iter()
                .any(|x| x.code == MediaIssueCode::Unreadable),
            "{name}"
        );
    }
}
