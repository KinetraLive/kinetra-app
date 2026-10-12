//! Media layers: playback math (pure, here) and the media library operations on `AppState`
//! (imports with progress and cancel, transport). How playback rides the live rules is in
//! `core::live` (Take starts it, Clear / PANIC stop it, Preview shows the in frame); the
//! out-point task and readiness are in `core::live_hub`.
//!
//! # Playback
//! A layer's [`MediaPlayback`] is Program state, never saved and never content: transport acts on
//! Program at once, in Studio mode too (like timers). Position at server time `t`:
//! `pos + (t - at)` while playing (`pos` before `at`), else `pos`; then the out point (`outMs`,
//! else the file's duration) clamps it (hold, clear) or wraps it into in..out (loop). Starts are
//! scheduled [`PLAY_LEAD_MS`] ahead so every renderer starts on the same server instant.

use crate::core::model::{
    MediaAction, MediaEndAction, MediaKind, MediaLayerConfig, MediaPlayback, PlaybackState,
};
use crate::core::protocol::WireMedia;

/// How far ahead of "now" a play is scheduled (every renderer starts at the same instant).
pub const PLAY_LEAD_MS: u64 = 120;

/// A media layer's content: its config and what the probe said about the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaContent {
    pub config: MediaLayerConfig,
    pub kind: MediaKind,
    /// The file's duration, when known.
    pub duration_ms: Option<u64>,
}

/// A stable `templateRev` for a media file: FNV-1a of the asset (a new file = a new rev).
pub fn media_rev(asset: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in asset.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

impl MediaContent {
    /// Whether the file has a timeline (video, audio). Images ignore transport.
    pub fn plays(&self) -> bool {
        self.kind != MediaKind::Image
    }

    /// The out point: `outMs`, else the file's duration (unknown: none).
    pub fn out_point(&self) -> Option<u64> {
        self.config.out_ms.or(self.duration_ms)
    }

    /// `ms` clamped into in..=out.
    fn clamp(&self, ms: u64) -> u64 {
        let ms = ms.max(self.config.in_ms);
        match self.out_point() {
            Some(out) => ms.min(out.max(self.config.in_ms)),
            None => ms,
        }
    }

    /// The media position at server time `now` (see the module docs).
    pub fn position(&self, pb: &MediaPlayback, now: u64) -> u64 {
        let raw = match pb.state {
            PlaybackState::Playing => pb.pos_ms + now.saturating_sub(pb.at),
            _ => pb.pos_ms,
        };
        let inp = self.config.in_ms;
        match (self.config.end_action, self.out_point()) {
            (MediaEndAction::Loop, Some(out)) if out > inp && raw >= out => {
                inp + (raw - inp) % (out - inp)
            }
            (_, Some(out)) => raw.min(out),
            (_, None) => raw,
        }
    }

    /// Server time at which playing reaches the out point, for hold and clear (loop never
    /// ends; an unknown out point never ends). `None` unless playing.
    pub fn end_at(&self, pb: &MediaPlayback) -> Option<u64> {
        if pb.state != PlaybackState::Playing
            || self.config.end_action == MediaEndAction::Loop
            || !self.plays()
        {
            return None;
        }
        let out = self.out_point()?;
        Some(pb.at + out.saturating_sub(pb.pos_ms))
    }

    /// Stopped at the in point.
    pub fn stopped(&self) -> MediaPlayback {
        MediaPlayback {
            state: PlaybackState::Stopped,
            at: 0,
            pos_ms: self.config.in_ms,
        }
    }

    /// What a Take starts: playing from the in point at `now + lead` (`autoplay`), or paused at
    /// the in point. Images: stopped (no timeline).
    pub fn take(&self, now: u64) -> MediaPlayback {
        if !self.plays() {
            return self.stopped();
        }
        if self.config.autoplay {
            self.play_from(self.config.in_ms, now)
        } else {
            MediaPlayback {
                state: PlaybackState::Paused,
                at: now,
                pos_ms: self.config.in_ms,
            }
        }
    }

    fn play_from(&self, pos: u64, now: u64) -> MediaPlayback {
        MediaPlayback {
            state: PlaybackState::Playing,
            at: now + PLAY_LEAD_MS,
            pos_ms: pos,
        }
    }

    /// Transport: the playback after `action` at `now` (`ms` for seek, clamped to in..out).
    /// Play on a stopped layer, or on one held at its out point, starts at the in point. Images
    /// ignore transport.
    pub fn control(
        &self,
        pb: &MediaPlayback,
        action: MediaAction,
        ms: Option<u64>,
        now: u64,
    ) -> MediaPlayback {
        if !self.plays() {
            return *pb;
        }
        match action {
            MediaAction::Play => match pb.state {
                PlaybackState::Playing => *pb,
                PlaybackState::Stopped => self.play_from(self.config.in_ms, now),
                PlaybackState::Paused => {
                    let pos = self.clamp(pb.pos_ms);
                    let at_end = self.out_point().is_some_and(|o| pos >= o)
                        && self.config.end_action != MediaEndAction::Loop;
                    self.play_from(if at_end { self.config.in_ms } else { pos }, now)
                }
            },
            MediaAction::Pause => match pb.state {
                PlaybackState::Playing => MediaPlayback {
                    state: PlaybackState::Paused,
                    at: now,
                    pos_ms: self.position(pb, now),
                },
                _ => *pb,
            },
            MediaAction::Stop => self.stopped(),
            MediaAction::Restart => self.play_from(self.config.in_ms, now),
            MediaAction::Seek => {
                let t = self.clamp(ms.unwrap_or(self.config.in_ms));
                match pb.state {
                    PlaybackState::Playing => self.play_from(t, now),
                    _ => MediaPlayback {
                        state: PlaybackState::Paused,
                        at: now,
                        pos_ms: t,
                    },
                }
            }
        }
    }

    /// Held at the out point (hold's end), as of `at`.
    pub fn held(&self, at: u64) -> MediaPlayback {
        MediaPlayback {
            state: PlaybackState::Paused,
            at,
            pos_ms: self.out_point().unwrap_or(self.config.in_ms),
        }
    }

    /// What renderers get (`WireLayer.media`).
    pub fn wire(&self, pb: MediaPlayback) -> WireMedia {
        WireMedia {
            url: self.config.asset.clone(),
            kind: self.kind,
            fit: self.config.fit,
            in_ms: self.config.in_ms,
            out_ms: self.config.out_ms,
            duration_ms: self.duration_ms,
            volume: self.config.volume,
            muted: self.config.muted,
            end_action: self.config.end_action,
            playback: pb,
        }
    }
}

/// A screen `main` (1920×1080) with one media layer `clip` (`/assets/clip.mp4`, in 0, the
/// given duration and end action), for tests.
#[cfg(test)]
pub(crate) fn test_plan(
    end_action: MediaEndAction,
    duration_ms: Option<u64>,
) -> crate::core::live::ScreenPlan {
    use crate::core::live::{media_template, LayerContent, LayerPlan, ScreenPlan};
    use crate::core::model::MediaFit;
    use crate::core::protocol::{ContentSizing, Fields, Rect, ScreenInfo};
    ScreenPlan {
        info: ScreenInfo {
            id: "main".into(),
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        },
        layers: vec![LayerPlan {
            id: "clip".into(),
            name: "Clip".into(),
            content: LayerContent {
                template: media_template(),
                variant: String::new(),
                fields: Fields::new(),
                variables: Vec::new(),
                var_values: Fields::new(),
                vars: Fields::new(),
                auto_out_ms: None,
                rect: Some(Rect {
                    x: 0,
                    y: 0,
                    w: 1920,
                    h: 1080,
                }),
                sizing: ContentSizing::Reflow,
                opacity: 100,
                media: Some(MediaContent {
                    config: MediaLayerConfig {
                        asset: "/assets/clip.mp4".into(),
                        fit: MediaFit::Fit,
                        in_ms: 0,
                        out_ms: None,
                        volume: 100,
                        muted: false,
                        end_action,
                        autoplay: true,
                    },
                    kind: MediaKind::Video,
                    duration_ms,
                }),
            },
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::MediaFit;

    fn clip(end: MediaEndAction) -> MediaContent {
        MediaContent {
            config: MediaLayerConfig {
                asset: "/assets/a.mp4".into(),
                fit: MediaFit::Fit,
                in_ms: 1000,
                out_ms: Some(5000),
                volume: 100,
                muted: false,
                end_action: end,
                autoplay: true,
            },
            kind: MediaKind::Video,
            duration_ms: Some(10_000),
        }
    }

    #[test]
    fn take_schedules_ahead_from_the_in_point() {
        let c = clip(MediaEndAction::Hold);
        let pb = c.take(10_000);
        assert_eq!(pb.state, PlaybackState::Playing);
        assert_eq!((pb.at, pb.pos_ms), (10_120, 1000));
        // Before `at` the position holds at the in point.
        assert_eq!(c.position(&pb, 10_050), 1000);
        assert_eq!(c.position(&pb, 11_120), 2000);
        // Hold clamps at the out point; it ends at at + (out - pos).
        assert_eq!(c.position(&pb, 99_999), 5000);
        assert_eq!(c.end_at(&pb), Some(14_120));
        let mut paused = c.clone();
        paused.config.autoplay = false;
        let pb = paused.take(10_000);
        assert_eq!((pb.state, pb.pos_ms), (PlaybackState::Paused, 1000));
        assert_eq!(paused.end_at(&pb), None);
    }

    #[test]
    fn loop_wraps_and_never_ends() {
        let c = clip(MediaEndAction::Loop);
        let pb = c.take(0);
        assert_eq!(c.position(&pb, 120 + 4000), 1000);
        assert_eq!(c.position(&pb, 120 + 4500), 1500);
        assert_eq!(c.end_at(&pb), None);
    }

    #[test]
    fn transport() {
        let c = clip(MediaEndAction::Hold);
        let pb = c.take(0);
        let paused = c.control(&pb, MediaAction::Pause, None, 1120);
        assert_eq!((paused.state, paused.pos_ms), (PlaybackState::Paused, 2000));
        assert_eq!(c.control(&paused, MediaAction::Pause, None, 5000), paused);
        let resumed = c.control(&paused, MediaAction::Play, None, 3000);
        assert_eq!((resumed.at, resumed.pos_ms), (3120, 2000));
        let seek = c.control(&resumed, MediaAction::Seek, Some(99_999), 4000);
        assert_eq!((seek.state, seek.pos_ms), (PlaybackState::Playing, 5000));
        let seek = c.control(&paused, MediaAction::Seek, Some(10), 4000);
        assert_eq!((seek.state, seek.pos_ms), (PlaybackState::Paused, 1000));
        let stop = c.control(&resumed, MediaAction::Stop, None, 4000);
        assert_eq!(stop, c.stopped());
        let play = c.control(&stop, MediaAction::Play, None, 5000);
        assert_eq!((play.at, play.pos_ms), (5120, 1000));
        // Play at the held end starts over.
        let held = c.held(9000);
        assert_eq!(held.pos_ms, 5000);
        assert_eq!(c.control(&held, MediaAction::Play, None, 9000).pos_ms, 1000);
        let restart = c.control(&paused, MediaAction::Restart, None, 7000);
        assert_eq!(
            (restart.state, restart.pos_ms),
            (PlaybackState::Playing, 1000)
        );
        // Images have no timeline.
        let mut img = c.clone();
        img.kind = MediaKind::Image;
        assert_eq!(img.take(0), img.stopped());
        assert_eq!(
            img.control(&img.stopped(), MediaAction::Play, None, 0),
            img.stopped()
        );
    }

    #[test]
    fn rev_is_stable_per_file() {
        assert_eq!(media_rev("/assets/a.mp4"), media_rev("/assets/a.mp4"));
        assert_ne!(media_rev("/assets/a.mp4"), media_rev("/assets/b.mp4"));
        assert!(media_rev("") >= 1);
    }
}
