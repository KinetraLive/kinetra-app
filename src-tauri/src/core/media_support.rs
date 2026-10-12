//! Which media plays where: one table, turned into [`MediaIssue`]s for a probed file.
//!
//! Runtimes: the app's WebView2 (Windows: display outputs and previews), WKWebView (macOS), and
//! OBS / vMix browser sources (Chromium/CEF). Verdicts:
//!
//! | Media | Windows | macOS | OBS | vMix |
//! |---|---|---|---|---|
//! | MP4/MOV H.264 (+ AAC) | ok¹ | ok | ok | ok |
//! | MP4/MOV HEVC | warning: needs the HEVC extension | ok | error | – |
//! | HEVC with alpha (MOV) | error | ok | error | – |
//! | ProRes | error | ok | error | error |
//! | WebM VP8 / VP9 | ok | warning: macOS 11.3+ | ok | ok |
//! | WebM VP8 / VP9 with alpha | ok | error: no alpha (on black) | ok | ok |
//! | AV1 (MP4 / WebM) | warning | warning: newer Macs only | warning | warning |
//! | Other MP4 video (MPEG-4 part 2, …) | error | error | error | error |
//! | MP4/MOV, index at the end | warning: starts slowly (all) | | | |
//! | AC-3 / E-AC-3 audio | error: no sound (all but macOS) | | | |
//! | MP3, WAV, AAC, M4A | ok | ok | ok | ok |
//!
//! ¹ Windows "N" editions need the Media Feature Pack for H.264/AAC; that is not shown as an
//! issue (it applies to every file and is a system setup step).
//!
//! Each issue carries a conversion with a copyable `ffmpeg` command for that file (quoted file
//! names; the output gets a suffix so it never overwrites the input). Kinetra doesn't bundle a
//! converter.

use crate::core::model::{
    IssueSeverity, MediaConvert, MediaInfo, MediaIssue, MediaIssueCode, MediaKind,
    MediaPlatform::{self, Macos, Obs, Vmix, Windows},
};

const ALL: [MediaPlatform; 4] = [Windows, Macos, Obs, Vmix];

fn issue(
    code: MediaIssueCode,
    severity: IssueSeverity,
    platforms: &[MediaPlatform],
    message: &str,
    convert: Option<MediaConvert>,
) -> MediaIssue {
    MediaIssue {
        code,
        severity,
        platforms: platforms.to_vec(),
        message: message.to_string(),
        convert,
    }
}

/// The issue of a file that couldn't be read.
pub fn unreadable() -> MediaIssue {
    issue(
        MediaIssueCode::Unreadable,
        IssueSeverity::Error,
        &ALL,
        "Can't read this file. It may be damaged or not a supported type.",
        None,
    )
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn to_h264(name: &str) -> MediaConvert {
    MediaConvert {
        label: "Convert to H.264".into(),
        command: format!(
            "ffmpeg -i \"{name}\" -c:v libx264 -crf 18 -pix_fmt yuv420p -c:a aac -movflags +faststart \"{}-h264.mp4\"",
            stem(name)
        ),
    }
}

fn to_vp9_alpha(name: &str) -> MediaConvert {
    MediaConvert {
        label: "Convert to WebM VP9 with alpha".into(),
        command: format!(
            "ffmpeg -i \"{name}\" -c:v libvpx-vp9 -pix_fmt yuva420p -b:v 0 -crf 30 -an \"{}-alpha.webm\"",
            stem(name)
        ),
    }
}

fn to_hevc_alpha(name: &str) -> MediaConvert {
    MediaConvert {
        label: "Convert to HEVC with alpha (.mov) for Mac (run it on a Mac)".into(),
        command: format!(
            "ffmpeg -i \"{name}\" -c:v hevc_videotoolbox -alpha_quality 0.75 -vtag hvc1 \"{}-alpha.mov\"",
            stem(name)
        ),
    }
}

fn faststart(name: &str) -> MediaConvert {
    MediaConvert {
        label: "Re-save with fast start".into(),
        command: format!(
            "ffmpeg -i \"{name}\" -c copy -movflags +faststart \"{}-faststart.mp4\"",
            stem(name)
        ),
    }
}

/// The base codec name ("h264 High 4.1" → "h264").
fn base(codec: &Option<String>) -> &str {
    codec
        .as_deref()
        .and_then(|c| c.split(' ').next())
        .unwrap_or("")
}

/// Issues of a probed file (`name` is its file name in `assets/`, used in the commands).
pub fn issues(info: &MediaInfo, name: &str) -> Vec<MediaIssue> {
    use IssueSeverity::{Error, Warning};
    use MediaIssueCode::*;
    let mut out = Vec::new();
    if info.kind == MediaKind::Image {
        return out;
    }
    let isobmff = matches!(info.container.as_str(), "mp4" | "mov");
    let video = base(&info.video_codec);
    match video {
        "" => {}
        "h264" => {}
        "hevc" if info.alpha => out.push(issue(
            Unsupported,
            Error,
            &[Windows, Obs],
            "Won't play on Windows or in OBS: HEVC with alpha.",
            Some(to_vp9_alpha(name)),
        )),
        "hevc" => {
            out.push(issue(
                NeedsExtension,
                Warning,
                &[Windows],
                "Needs the HEVC Video Extensions on Windows.",
                Some(to_h264(name)),
            ));
            out.push(issue(
                Unsupported,
                Error,
                &[Obs],
                "Won't play in OBS: HEVC.",
                Some(to_h264(name)),
            ));
        }
        "prores" => out.push(issue(
            Unsupported,
            Error,
            &[Windows, Obs, Vmix],
            "Won't play on Windows or in browser sources: ProRes.",
            Some(if info.alpha {
                to_vp9_alpha(name)
            } else {
                to_h264(name)
            }),
        )),
        "vp8" | "vp9" if info.alpha => out.push(issue(
            NoAlpha,
            Error,
            &[Macos],
            "Shows on black on Mac.",
            Some(to_hevc_alpha(name)),
        )),
        "vp8" | "vp9" => out.push(issue(
            Unsupported,
            Warning,
            &[Macos],
            "Needs macOS 11.3 or later.",
            None,
        )),
        "av1" => {
            out.push(issue(
                NeedsExtension,
                Warning,
                &[Windows, Obs, Vmix],
                "May not play on older Windows or in browser sources: AV1.",
                Some(to_h264(name)),
            ));
            out.push(issue(
                Unsupported,
                Warning,
                &[Macos],
                "Plays only on newer Macs: AV1.",
                Some(to_h264(name)),
            ));
        }
        other => {
            let shown = match other {
                "mpeg4" => "MPEG-4 Part 2".to_string(),
                o => o.to_uppercase(),
            };
            out.push(issue(
                Unsupported,
                Error,
                &ALL,
                &format!("Won't play: {shown} video."),
                Some(to_h264(name)),
            ));
        }
    }
    if matches!(base(&info.audio_codec), "ac3" | "eac3") {
        out.push(issue(
            Unsupported,
            Error,
            &[Windows, Obs, Vmix],
            "No sound: AC-3 audio.",
            Some(to_h264(name)),
        ));
    }
    if isobmff && info.fast_start == Some(false) {
        out.push(issue(
            SlowStart,
            Warning,
            &ALL,
            "Starts slowly: the index is at the end of the file.",
            Some(faststart(name)),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(container: &str, video: Option<&str>, alpha: bool) -> MediaInfo {
        MediaInfo {
            kind: MediaKind::Video,
            container: container.into(),
            duration_ms: Some(1000),
            width: Some(1920),
            height: Some(1080),
            fps: Some(25.0),
            video_codec: video.map(str::to_string),
            audio_codec: Some("aac".into()),
            alpha,
            fast_start: matches!(container, "mp4" | "mov").then_some(true),
            issues: Vec::new(),
        }
    }

    fn verdict(i: &[MediaIssue], p: MediaPlatform) -> Option<IssueSeverity> {
        let mut worst = None;
        for x in i.iter().filter(|x| x.platforms.contains(&p)) {
            if x.severity == IssueSeverity::Error || worst.is_none() {
                worst = Some(x.severity);
            }
        }
        worst
    }

    #[test]
    fn table_verdicts_per_platform() {
        use IssueSeverity::{Error as E, Warning as W};
        // (container, codec, alpha) → windows, macos, obs, vmix
        let rows: &[(&str, &str, bool, [Option<IssueSeverity>; 4])] = &[
            ("mp4", "h264 High 4.1", false, [None, None, None, None]),
            ("mp4", "hevc", false, [Some(W), None, Some(E), None]),
            ("mov", "hevc", true, [Some(E), None, Some(E), None]),
            ("mov", "prores", true, [Some(E), None, Some(E), Some(E)]),
            ("webm", "vp9", false, [None, Some(W), None, None]),
            ("webm", "vp8", true, [None, Some(E), None, None]),
            ("webm", "vp9", true, [None, Some(E), None, None]),
            ("mp4", "av1", false, [Some(W), Some(W), Some(W), Some(W)]),
            ("mp4", "mpeg4", false, [Some(E), Some(E), Some(E), Some(E)]),
        ];
        for (c, v, a, want) in rows {
            let i = issues(&info(c, Some(v), *a), "clip.mp4");
            let got = [Windows, Macos, Obs, Vmix].map(|p| verdict(&i, p));
            assert_eq!(&got, want, "{c} {v} alpha={a}: {i:?}");
        }
    }

    #[test]
    fn audio_and_images_are_fine() {
        let mut a = info("mp3", None, false);
        a.kind = MediaKind::Audio;
        a.audio_codec = Some("mp3".into());
        assert!(issues(&a, "song.mp3").is_empty());
        let mut img = info("png", None, false);
        img.kind = MediaKind::Image;
        assert!(issues(&img, "a.png").is_empty());
    }

    #[test]
    fn commands_use_the_quoted_file_name() {
        let mut i = info("mp4", Some("h264 Main 3.1"), false);
        i.fast_start = Some(false);
        let is = issues(&i, "My clip.mp4");
        assert_eq!(is.len(), 1);
        assert_eq!(is[0].code, MediaIssueCode::SlowStart);
        assert_eq!(
            is[0].convert.as_ref().unwrap().command,
            "ffmpeg -i \"My clip.mp4\" -c copy -movflags +faststart \"My clip-faststart.mp4\""
        );
        let alpha = issues(&info("webm", Some("vp9"), true), "logo.webm");
        assert_eq!(alpha[0].code, MediaIssueCode::NoAlpha);
        assert_eq!(alpha[0].message, "Shows on black on Mac.");
        assert!(alpha[0]
            .convert
            .as_ref()
            .unwrap()
            .command
            .starts_with("ffmpeg -i \"logo.webm\" -c:v hevc_videotoolbox"));
        let prores = issues(&info("mov", Some("prores"), true), "key.mov");
        assert!(prores[0]
            .convert
            .as_ref()
            .unwrap()
            .command
            .contains("libvpx-vp9 -pix_fmt yuva420p"));
    }
}
