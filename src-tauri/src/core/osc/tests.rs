//! OSC address scheme, inputs, accept rule, patterns and the on-air diff.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::*;

fn cmd(id: &str) -> Option<OscAction> {
    Some(OscAction::Command(id.into()))
}

#[test]
fn addresses_map_to_the_registry_commands() {
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/take"),
        cmd("live.take:main/l1")
    );
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/clear"),
        cmd("live.clear:main/l1")
    );
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/toggle"),
        cmd("live.toggle:main/l1")
    );
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/update"),
        cmd("live.update:main/l1")
    );
    assert_eq!(parse_address("/kinetra/flow/f1/go"), cmd("cue.go:f1"));
    assert_eq!(
        parse_address("/kinetra/source/s1/next"),
        cmd("source.next:s1")
    );
    assert_eq!(
        parse_address("/kinetra/source/s1/prev"),
        cmd("source.prev:s1")
    );
    assert_eq!(
        parse_address("/kinetra/screen/main/clear"),
        cmd("live.clearScreen:main")
    );
    assert_eq!(parse_address("/kinetra/clear-all"), cmd("live.clearAll"));
    assert_eq!(parse_address("/kinetra/panic"), cmd("live.panic"));
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/opacity"),
        Some(OscAction::Value {
            screen_id: "main".into(),
            layer_id: "l1".into(),
            target: ValueTarget::Opacity,
        })
    );
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/var/radius"),
        Some(OscAction::Value {
            screen_id: "main".into(),
            layer_id: "l1".into(),
            target: ValueTarget::Var {
                key: "radius".into()
            },
        })
    );
    for bad in [
        "/kinetra/layer/main/take",
        "/kinetra/layer//l1/take",
        "/kinetra/layer/main/l1/take/",
        "/kinetra/layer/main/l1/explode",
        "/kinetra/flow/f1",
        "/kinetra/source/s1/first",
        "/kinetra/panic/now",
        "/kinetra",
        "/other/panic",
        "kinetra/panic",
    ] {
        assert_eq!(parse_address(bad), None, "{bad}");
    }
}

#[test]
fn a_trigger_release_does_nothing_and_levels_are_numbers() {
    let take = OscAction::Command("live.take:main/l1".into());
    assert_eq!(input_for(&take, &[]), Some(ControlInput::Press));
    assert_eq!(
        input_for(&take, &[OscArg::Int(1)]),
        Some(ControlInput::Press)
    );
    assert_eq!(
        input_for(&take, &[OscArg::Float(1.0)]),
        Some(ControlInput::Press)
    );
    assert_eq!(
        input_for(&take, &[OscArg::Text("go".into())]),
        Some(ControlInput::Press)
    );
    assert_eq!(input_for(&take, &[OscArg::Int(0)]), None);
    assert_eq!(input_for(&take, &[OscArg::Float(0.0)]), None);
    assert_eq!(input_for(&take, &[OscArg::Bool(false)]), None);

    let opacity = parse_address("/kinetra/layer/main/l1/opacity").unwrap();
    assert_eq!(input_for(&opacity, &[]), Some(ControlInput::Press));
    assert_eq!(
        input_for(&opacity, &[OscArg::Float(0.5)]),
        Some(ControlInput::Level(0.5))
    );
    assert_eq!(
        input_for(&opacity, &[OscArg::Double(0.25)]),
        Some(ControlInput::Level(0.25))
    );
    assert_eq!(
        input_for(&opacity, &[OscArg::Int(1)]),
        Some(ControlInput::Level(1.0))
    );
    assert_eq!(
        input_for(&opacity, &[OscArg::Text(" 0.75 ".into())]),
        Some(ControlInput::Level(0.75))
    );
    assert_eq!(input_for(&opacity, &[OscArg::Text("half".into())]), None);
    assert_eq!(input_for(&opacity, &[OscArg::Other]), None);
    assert_eq!(input_for(&opacity, &[OscArg::Double(f64::NAN)]), None);
}

#[test]
fn argument_text_is_short() {
    assert_eq!(OscArg::Float(0.1).text(), "0.1");
    assert_eq!(OscArg::Float(1.0).text(), "1");
    assert_eq!(OscArg::Int(-3).text(), "-3");
    assert_eq!(OscArg::Double(2.5).text(), "2.5");
    assert_eq!(OscArg::Bool(true).text(), "true");
    assert_eq!(OscArg::Text("Ada".into()).text(), "Ada");
    assert_eq!(OscArg::Other.text(), "");
}

#[test]
fn loopback_is_always_accepted_others_only_with_lan_and_the_allowlist() {
    let allow = parse_allow(&[
        "192.168.1.20".into(),
        " fe80::1 ".into(),
        "::ffff:10.0.0.5".into(),
        "not an ip".into(),
    ]);
    assert_eq!(allow.len(), 3);
    let v4 = |a, b, c, d| IpAddr::V4(Ipv4Addr::new(a, b, c, d));
    let lo4 = v4(127, 0, 0, 1);
    let lo6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
    let lan = v4(192, 168, 1, 20);
    let other = v4(192, 168, 1, 21);
    let mapped: IpAddr = "::ffff:192.168.1.20".parse().unwrap();
    let mapped_lo: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
    let ll: IpAddr = "fe80::1".parse().unwrap();

    for lan_on in [false, true] {
        assert!(accepts(lo4, lan_on, &allow));
        assert!(accepts(lo6, lan_on, &allow));
        assert!(accepts(mapped_lo, lan_on, &allow));
        assert!(accepts(v4(127, 0, 0, 2), lan_on, &allow));
        assert!(accepts(lo4, lan_on, &[]));
    }
    // LAN access off: nothing from other machines, allowlisted or not.
    assert!(!accepts(lan, false, &allow));
    assert!(!accepts(mapped, false, &allow));
    // LAN access on: only the allowlist (IPv4-mapped IPv6 compared as IPv4, both ways).
    assert!(accepts(lan, true, &allow));
    assert!(accepts(mapped, true, &allow));
    assert!(accepts(ll, true, &allow));
    assert!(accepts(v4(10, 0, 0, 5), true, &allow));
    assert!(!accepts(other, true, &allow));
    assert!(!accepts(lan, true, &[]));
}

#[test]
fn patterns_match_whole_segments() {
    assert!(address_matches("/stage/go", "/stage/go"));
    assert!(address_matches("/stage/*/go", "/stage/a/go"));
    assert!(address_matches("/*/*", "/a/b"));
    assert!(!address_matches("/stage/*/go", "/stage/a/b/go"));
    assert!(!address_matches("/stage/*/go", "/stage/go"));
    assert!(!address_matches("/stage/go", "/stage/go/"));
    assert!(!address_matches("/stage/g*", "/stage/go"));
    assert!(has_wildcard("/stage/*/go"));
    assert!(!has_wildcard("/stage/g*"));

    assert!(is_osc_pattern("/stage/go"));
    assert!(is_osc_pattern("/kinetra/layer/*/*/onair"));
    assert!(!is_osc_pattern(""));
    assert!(!is_osc_pattern("/"));
    assert!(!is_osc_pattern("stage/go"));
    assert!(!is_osc_pattern("/stage//go"));
    assert!(!is_osc_pattern("/stage/go/"));
    assert!(!is_osc_pattern("/stage go"));
    assert!(is_osc_pattern(&format!("/{}", "a".repeat(255))));
    assert!(!is_osc_pattern(&format!("/{}", "a".repeat(256))));
}

#[test]
fn the_diff_sends_everything_once_then_changes() {
    let l = |s: &str, l: &str, on| (s.to_string(), l.to_string(), on);
    let mut d = OnAirDiff::default();
    let first = d.update(vec![l("m", "a", false), l("m", "b", true)]);
    assert_eq!(first, vec![l("m", "a", false), l("m", "b", true)]);
    assert!(d
        .update(vec![l("m", "a", false), l("m", "b", true)])
        .is_empty());
    assert_eq!(
        d.update(vec![l("m", "a", true), l("m", "b", true)]),
        vec![l("m", "a", true)]
    );
    // A removed on-air layer is sent as off; a removed off-air one is just forgotten.
    let mut gone = d.update(vec![l("m", "c", false)]);
    gone.sort();
    assert_eq!(
        gone,
        vec![l("m", "a", false), l("m", "b", false), l("m", "c", false)]
    );
    d.update(vec![l("m", "c", false), l("m", "d", true)]);
    assert_eq!(d.update(vec![l("m", "c", false)]), vec![l("m", "d", false)]);
    assert!(d.update(vec![]).is_empty());
    d.reset();
    assert_eq!(d.update(vec![l("m", "c", false)]), vec![l("m", "c", false)]);
}

#[test]
fn media_transport_addresses() {
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/play"),
        cmd("media.play:main/l1")
    );
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/pause"),
        cmd("media.pause:main/l1")
    );
    assert_eq!(
        parse_address("/kinetra/layer/main/l1/stop"),
        cmd("media.stop:main/l1")
    );
    let seek = parse_address("/kinetra/layer/main/l1/seek").unwrap();
    assert_eq!(
        seek,
        OscAction::Seek {
            screen_id: "main".into(),
            layer_id: "l1".into()
        }
    );
    assert!(input_for(&seek, &[OscArg::Float(2.5)]).is_some());
    assert!(input_for(&seek, &[]).is_none(), "a seek needs a position");
    use crate::core::control_actions::{parse_command, ControlCommand};
    assert_eq!(
        parse_command("media.pause:main/l1"),
        Some(ControlCommand::Media {
            screen: "main",
            layer: "l1",
            action: MediaAction::Pause
        })
    );
    assert_eq!(parse_command("media.seek:main/l1"), None);
}
