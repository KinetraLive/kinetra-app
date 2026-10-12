//! Media layers in the hub: Take starts playback, transport, end actions, PANIC, Preview.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::broadcast::error::TryRecvError;

use super::*;
use crate::core::live::LayerContent;
use crate::core::media::{test_plan, PLAY_LEAD_MS};
use crate::core::model::{LiveEdit, MediaAction, MediaEndAction, PlaybackState};

fn json(m: &HubMsg) -> Value {
    serde_json::from_str(&m.json).unwrap()
}

fn hub_with(end: MediaEndAction, duration: Option<u64>) -> LiveHub {
    let hub = LiveHub::default();
    hub.apply_screen(test_plan(end, duration), LiveEdit::OnTake)
        .unwrap();
    hub
}

fn playback(hub: &LiveHub) -> crate::core::model::MediaPlayback {
    hub.layer_view("main", "clip")
        .unwrap()
        .media
        .unwrap()
        .playback
}

fn drain(rx: &mut broadcast::Receiver<Arc<HubMsg>>) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(m) if !m.preview => out.push(json(&m)),
            Ok(_) => {}
            Err(_) => return out,
        }
    }
}

#[test]
fn wire_layer_and_rev_from_the_file() {
    let hub = hub_with(MediaEndAction::Hold, Some(5000));
    let snap: Value = serde_json::from_str(&hub.snapshot_json("main").unwrap()).unwrap();
    let l = &snap["layers"][0];
    assert_eq!(l["html"], "");
    assert_eq!(l["fields"], serde_json::json!({}));
    assert_eq!(
        l["templateRev"],
        crate::core::media::media_rev("/assets/clip.mp4")
    );
    let m = &l["media"];
    assert_eq!(m["url"], "/assets/clip.mp4");
    assert_eq!(m["kind"], "video");
    assert_eq!(m["fit"], "fit");
    assert_eq!(m["durationMs"], 5000);
    assert_eq!(m["endAction"], "hold");
    assert_eq!(m["playback"]["state"], "stopped");
    assert!(m.get("outMs").is_none());
}

#[test]
fn take_plays_from_the_in_point_scheduled_ahead() {
    let hub = hub_with(MediaEndAction::Loop, Some(5000));
    let mut rx = hub.subscribe();
    let before = fsx::now_epoch_ms();
    hub.show("main", "clip").unwrap();
    let msgs = drain(&mut rx);
    assert_eq!(msgs[0]["t"], "state");
    assert_eq!(msgs[1]["t"], "media");
    assert_eq!(msgs[1]["playback"]["state"], "playing");
    assert_eq!(msgs[1]["playback"]["posMs"], 0);
    let at = msgs[1]["playback"]["at"].as_u64().unwrap();
    assert!(at >= before + PLAY_LEAD_MS && at <= fsx::now_epoch_ms() + PLAY_LEAD_MS);
    // Loop never ends: nothing scheduled.
    assert_eq!(hub.pending_media_ends(), 0);
    // A reconnect snapshot carries the same playback.
    let snap: Value = serde_json::from_str(&hub.snapshot_json("main").unwrap()).unwrap();
    assert_eq!(snap["layers"][0]["media"]["playback"]["at"], at);
    // Clear: state out only, playback stopped (renderers stop after the out-animation).
    hub.hide("main", "clip").unwrap();
    let msgs = drain(&mut rx);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert_eq!(msgs[0]["state"], "out");
    assert_eq!(playback(&hub).state, PlaybackState::Stopped);
}

#[test]
fn transport_acts_on_program_and_rejects_templates() {
    let hub = hub_with(MediaEndAction::Loop, Some(5000));
    hub.show("main", "clip").unwrap();
    let mut rx = hub.subscribe();
    hub.media_control("main", "clip", MediaAction::Pause, None)
        .unwrap();
    assert_eq!(playback(&hub).state, PlaybackState::Paused);
    hub.media_control("main", "clip", MediaAction::Seek, Some(2500))
        .unwrap();
    assert_eq!(playback(&hub).pos_ms, 2500);
    hub.media_control("main", "clip", MediaAction::Pause, None)
        .unwrap();
    let msgs = drain(&mut rx);
    assert_eq!(msgs.len(), 2, "a no-op sends nothing: {msgs:?}");
    assert!(msgs.iter().all(|m| m["t"] == "media"));
    // Studio mode: transport still acts on Program at once.
    hub.set_studio(true, None, TakeTransition::default())
        .unwrap();
    hub.media_control("main", "clip", MediaAction::Play, None)
        .unwrap();
    assert_eq!(playback(&hub).state, PlaybackState::Playing);
    let mut live = LiveState::with_test_screen();
    live.studio = false;
    let t = LiveHub::new(live);
    assert!(matches!(
        t.media_control(
            crate::core::live::TEST_SCREEN_ID,
            crate::core::live::TEST_LAYER_ID,
            MediaAction::Play,
            None
        ),
        Err(LiveError::NotMedia(_))
    ));
}

#[test]
fn panic_stops_and_cancels_the_end() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let hub = hub_with(MediaEndAction::Hold, Some(60_000));
    hub.set_runtime(rt.handle().clone());
    hub.show("main", "clip").unwrap();
    assert_eq!(hub.pending_media_ends(), 1);
    // Pause cancels, play reschedules.
    hub.media_control("main", "clip", MediaAction::Pause, None)
        .unwrap();
    assert_eq!(hub.pending_media_ends(), 0);
    hub.media_control("main", "clip", MediaAction::Play, None)
        .unwrap();
    assert_eq!(hub.pending_media_ends(), 1);
    hub.panic();
    assert_eq!(hub.pending_media_ends(), 0);
    assert_eq!(playback(&hub).state, PlaybackState::Stopped);
    // Unknown duration: nothing to schedule.
    let hub = hub_with(MediaEndAction::Clear, None);
    hub.set_runtime(rt.handle().clone());
    hub.show("main", "clip").unwrap();
    assert_eq!(hub.pending_media_ends(), 0);
}

#[tokio::test]
async fn hold_pauses_at_the_out_point_and_fires_media_ended() {
    let hub = hub_with(MediaEndAction::Hold, Some(150));
    let ended = Arc::new(Mutex::new(Vec::new()));
    let e = ended.clone();
    hub.set_media_ended_hook(Arc::new(move |s: &str, l: &str| {
        e.lock().unwrap().push(format!("{s}/{l}"));
    }));
    let mut rx = hub.subscribe();
    hub.show("main", "clip").unwrap();
    for _ in 0..200 {
        if playback(&hub).state == PlaybackState::Paused {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pb = playback(&hub);
    assert_eq!((pb.state, pb.pos_ms), (PlaybackState::Paused, 150));
    assert!(hub.is_on_air("main", "clip"), "hold stays on air");
    assert_eq!(ended.lock().unwrap().as_slice(), ["main/clip"]);
    assert_eq!(hub.pending_media_ends(), 0);
    let msgs = drain(&mut rx);
    assert_eq!(msgs.last().unwrap()["t"], "media");
    assert_eq!(msgs.last().unwrap()["playback"]["state"], "paused");
}

#[tokio::test]
async fn clear_takes_the_layer_off_air_at_the_out_point() {
    let hub = hub_with(MediaEndAction::Clear, Some(100));
    let ended = Arc::new(Mutex::new(0));
    let e = ended.clone();
    hub.set_media_ended_hook(Arc::new(move |_: &str, _: &str| {
        *e.lock().unwrap() += 1;
    }));
    hub.show("main", "clip").unwrap();
    for _ in 0..200 {
        if !hub.is_on_air("main", "clip") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!hub.is_on_air("main", "clip"));
    assert_eq!(*ended.lock().unwrap(), 1);
    assert_eq!(playback(&hub).state, PlaybackState::Stopped);
    // A pause before the end: nothing fires.
    hub.show("main", "clip").unwrap();
    hub.media_control("main", "clip", MediaAction::Pause, None)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(hub.is_on_air("main", "clip"));
    assert_eq!(*ended.lock().unwrap(), 1);
}

#[test]
fn preview_bus_shows_media_paused_at_the_in_point() {
    let hub = hub_with(MediaEndAction::Hold, Some(5000));
    let mut rx = hub.subscribe();
    let (_sub, snap) = hub.preview_attach("main");
    let snap: Value = serde_json::from_str(&snap.unwrap()).unwrap();
    assert_eq!(snap["layers"][0]["media"]["playback"]["state"], "paused");
    hub.show("main", "clip").unwrap();
    hub.media_control("main", "clip", MediaAction::Pause, None)
        .unwrap();
    // Program transport never reaches the Preview bus.
    loop {
        match rx.try_recv() {
            Ok(m) if m.preview => assert_ne!(json(&m)["t"], "media", "{}", m.json),
            Ok(_) => {}
            Err(TryRecvError::Empty) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
}

#[test]
fn a_new_file_on_air_restarts_and_config_changes_go_in_place() {
    let hub = hub_with(MediaEndAction::Hold, Some(5000));
    hub.show("main", "clip").unwrap();
    let rev = hub.with_layer("main", "clip", |l| l.template_rev).unwrap();
    let mut rx = hub.subscribe();
    // Volume change (immediate live edit): a same-rev layer message.
    let mut plan = test_plan(MediaEndAction::Hold, Some(5000));
    let edit = |p: &mut crate::core::live::ScreenPlan, f: &dyn Fn(&mut LayerContent)| {
        f(&mut p.layers[0].content)
    };
    edit(&mut plan, &|c| c.media.as_mut().unwrap().config.volume = 40);
    hub.apply_screen(plan, LiveEdit::Immediate).unwrap();
    let msgs = drain(&mut rx);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert_eq!(msgs[0]["t"], "layer");
    assert_eq!(msgs[0]["layer"]["templateRev"], rev);
    assert_eq!(msgs[0]["layer"]["media"]["volume"], 40);
    // A new file: a new rev, playing from the in point.
    let mut plan = test_plan(MediaEndAction::Hold, Some(5000));
    edit(&mut plan, &|c| {
        c.media.as_mut().unwrap().config.asset = "/assets/other.webm".into()
    });
    hub.apply_screen(plan, LiveEdit::Immediate).unwrap();
    let msgs = drain(&mut rx);
    assert_eq!(msgs[0]["t"], "layer");
    assert_ne!(msgs[0]["layer"]["templateRev"], rev);
    assert_eq!(msgs[0]["layer"]["media"]["playback"]["state"], "playing");
}

/// A screen with one template layer that has a video field (and a text field).
fn template_video_hub(video: &str) -> LiveHub {
    use crate::core::live::{FieldDef, TemplateDef};
    use crate::core::protocol::FieldType;
    let mut plan = test_plan(MediaEndAction::Hold, Some(3000));
    let l = &mut plan.layers[0];
    l.id = "tpl".into();
    l.content.media = None;
    l.content.template = Arc::new(TemplateDef {
        html: "<div><video data-bind-src=\"clip\"></video>{{title}}</div>".into(),
        css: String::new(),
        fields: vec![
            FieldDef {
                key: "clip".into(),
                label: "Clip".into(),
                kind: FieldType::Video,
                ..FieldDef::default()
            },
            FieldDef {
                key: "title".into(),
                label: "Title".into(),
                ..FieldDef::default()
            },
        ],
        design: None,
    });
    l.content.fields = [("clip".into(), video.into()), ("title".into(), "A".into())].into();
    let hub = LiveHub::default();
    hub.apply_screen(plan, LiveEdit::Immediate).unwrap();
    hub
}

#[test]
fn template_layers_with_video_fields_report_readiness() {
    let hub = template_video_hub("/assets/a.webm");
    let ready = |hub: &LiveHub| hub.layer_view("main", "tpl").unwrap().readiness;
    // No renderer yet: 0 of 0; media layers' `media` stays absent for templates.
    assert_eq!(hub.layer_view("main", "tpl").unwrap().media, None);
    assert_eq!(ready(&hub).map(|r| (r.ready, r.total)), Some((0, 0)));
    let reg = hub.renderer_attach("main", true);
    let preview = hub.renderer_attach("main", false);
    assert_eq!(ready(&hub).map(|r| (r.ready, r.total)), Some((0, 1)));
    hub.renderer_ready(&preview, "tpl", true, None);
    assert_eq!(
        ready(&hub).map(|r| r.ready),
        Some(0),
        "app previews never count"
    );
    hub.renderer_ready(&reg, "tpl", true, None);
    assert_eq!(ready(&hub).map(|r| (r.ready, r.total)), Some((1, 1)));
    // A text edit keeps the report; a new file needs a new one.
    hub.set_fields("main", "tpl", [("title".into(), "B".into())].into())
        .unwrap();
    assert_eq!(ready(&hub).map(|r| r.ready), Some(1));
    hub.set_fields(
        "main",
        "tpl",
        [("clip".into(), "/assets/b.webm".into())].into(),
    )
    .unwrap();
    assert_eq!(
        ready(&hub).map(|r| r.ready),
        Some(0),
        "a report for the old file"
    );
    hub.renderer_ready(&reg, "tpl", false, Some("Format not supported".into()));
    let r = ready(&hub).unwrap();
    assert_eq!(
        (r.ready, r.error.as_deref()),
        (0, Some("Format not supported"))
    );
    drop(reg);
    assert_eq!(ready(&hub).map(|r| r.total), Some(0));
}

#[test]
fn template_layers_without_media_have_no_readiness() {
    let hub = template_video_hub("");
    assert_eq!(hub.layer_view("main", "tpl").unwrap().readiness, None);
    let reg = hub.renderer_attach("main", true);
    hub.renderer_ready(&reg, "tpl", true, None);
    assert_eq!(hub.layer_view("main", "tpl").unwrap().readiness, None);
}
