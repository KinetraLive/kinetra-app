//! The app ↔ server messages of `docs/CONTROL_PROTOCOL.md` (protocol 1) and its protocol 2 delta
//! (kinetra-cloud `docs/CONTROL_PROTOCOL_V2.md`), close codes and limits. Unknown message types from the server are ignored (§2); unknown fields too.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::perms::Perms;

/// The highest protocol this app speaks.
pub const PROTOCOL: u32 = 2;
/// What the upgrade offers: protocol 2 first, protocol 1 for servers without it (§1 of
/// protocol 2). The server echoes the one it picked.
pub const SUBPROTOCOLS: &str = "kinetra-control.v2, kinetra-control.v1";

/// The protocol a subprotocol the server echoed stands for (`None`: not one we offered).
pub fn protocol_of(subprotocol: &str) -> Option<u32> {
    match subprotocol.trim() {
        "kinetra-control.v2" => Some(2),
        "kinetra-control.v1" => Some(1),
        _ => None,
    }
}
/// The app endpoint, under the API base.
pub const APP_PATH: &str = "/v1/control/app";

/// §11 defaults; the server's values arrive in `welcome.limits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub snapshot_bytes: usize,
    pub diff_bytes: usize,
    pub diffs_per_s: u32,
    pub snapshots_per_s: u32,
    pub result_timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            snapshot_bytes: 2 * 1024 * 1024,
            diff_bytes: 256 * 1024,
            diffs_per_s: 60,
            snapshots_per_s: 1,
            result_timeout_ms: 5000,
        }
    }
}

/// Most ops in one diff (§7).
pub const DIFF_OPS_MAX: usize = 256;
/// §11 counts.
pub const SCREENS_MAX: usize = 32;
pub const LAYERS_MAX: usize = 1000;
pub const TEMPLATES_MAX: usize = 200;
pub const CUES_MAX: usize = 500;
pub const ASSETS_MAX: usize = 1000;
pub const NAME_MAX: usize = 100;
pub const LIST_VALUE_MAX: usize = 64 * 1024;
pub const VALUE_MAX: usize = 16 * 1024;
/// Keys in one `update_fields` (§8).
pub const VALUES_MAX: usize = 64;
/// A result's message (§8).
pub const MESSAGE_MAX: usize = 200;

/// App → server.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Out {
    Hello {
        protocol: u32,
        app_version: String,
        machine: String,
        license_token: String,
        nonce: String,
        timestamp: i64,
    },
    Register {
        corr: String,
        project_id: String,
        name: String,
        resume_run: Option<String>,
        takeover: bool,
    },
    Snapshot {
        seq: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        corr: Option<String>,
        state: Value,
    },
    Diff {
        seq: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cause: Option<String>,
        ops: Vec<Op>,
    },
    Result(CmdResult),
    Ping {
        corr: String,
        sent_ms: u64,
    },
    Pong {
        corr: String,
        ping_sent_ms: u64,
        received_ms: u64,
        sent_ms: u64,
    },
    Rename {
        name: String,
    },
    End {
        reason: EndReason,
    },
}

impl Out {
    /// The `t` of the message (diagnostics).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::Register { .. } => "register",
            Self::Snapshot { .. } => "snapshot",
            Self::Diff { .. } => "diff",
            Self::Result(_) => "result",
            Self::Ping { .. } => "ping",
            Self::Pong { .. } => "pong",
            Self::Rename { .. } => "rename",
            Self::End { .. } => "end",
        }
    }

    pub fn corr(&self) -> Option<&str> {
        match self {
            Self::Register { corr, .. } | Self::Ping { corr, .. } | Self::Pong { corr, .. } => {
                Some(corr)
            }
            Self::Snapshot { corr, .. } => corr.as_deref(),
            Self::Diff { cause, .. } => cause.as_deref(),
            Self::Result(r) => Some(&r.corr),
            _ => None,
        }
    }

    pub fn seq(&self) -> Option<u64> {
        match self {
            Self::Snapshot { seq, .. } | Self::Diff { seq, .. } => Some(*seq),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Disabled,
    ProjectClosed,
    Quit,
    ProbeDone,
    Other,
}

/// One JSON Pointer operation of a diff (§7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Put { path: String, value: Value },
    Del { path: String },
}

impl Op {
    pub fn path(&self) -> &str {
        match self {
            Self::Put { path, .. } | Self::Del { path } => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmdResult {
    pub corr: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_ms: Option<u64>,
}

/// Server → app. Anything else is `Unknown` and ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum In {
    Welcome {
        #[serde(default)]
        protocol: u32,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        server_version: String,
        #[serde(default)]
        server_time_ms: u64,
        #[serde(default = "default_heartbeat")]
        heartbeat_s: u64,
        #[serde(default)]
        limits: Limits,
        /// What the server has switched on (protocol 2 §1), e.g. `invites`.
        #[serde(default)]
        features: Vec<String>,
    },
    Registered {
        #[serde(default)]
        corr: Option<String>,
        show_id: String,
        run_id: String,
        #[serde(default)]
        resumed: bool,
        #[serde(default)]
        crew_url: String,
        #[serde(default)]
        controller_url: String,
    },
    Error {
        code: String,
        #[serde(default)]
        message: String,
        #[serde(default)]
        fatal: bool,
        #[serde(default)]
        corr: Option<String>,
        #[serde(default)]
        details: Option<Value>,
    },
    Resync {
        corr: String,
        #[serde(default)]
        reason: String,
    },
    Command(Command),
    Crew {
        #[serde(default)]
        connected: Vec<CrewMember>,
    },
    Ping {
        corr: String,
        #[serde(default)]
        sent_ms: u64,
    },
    Pong {
        corr: String,
        #[serde(default)]
        ping_sent_ms: u64,
        #[serde(default)]
        received_ms: u64,
        #[serde(default)]
        sent_ms: u64,
    },
    ServerRestart {
        #[serde(default)]
        retry_after_ms: u64,
    },
    LicenseWarning {
        #[serde(default)]
        ends_ms: u64,
        #[serde(default)]
        message: String,
    },
    #[serde(other)]
    Unknown,
}

fn default_heartbeat() -> u64 {
    20
}

impl In {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Welcome { .. } => "welcome",
            Self::Registered { .. } => "registered",
            Self::Error { .. } => "error",
            Self::Resync { .. } => "resync",
            Self::Command(_) => "command",
            Self::Crew { .. } => "crew",
            Self::Ping { .. } => "ping",
            Self::Pong { .. } => "pong",
            Self::ServerRestart { .. } => "server_restart",
            Self::LicenseWarning { .. } => "license_warning",
            Self::Unknown => "unknown",
        }
    }

    pub fn corr(&self) -> Option<&str> {
        match self {
            Self::Registered { corr, .. } | Self::Error { corr, .. } => corr.as_deref(),
            Self::Resync { corr, .. } | Self::Ping { corr, .. } | Self::Pong { corr, .. } => {
                Some(corr)
            }
            Self::Command(c) => Some(&c.corr),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Command {
    pub corr: String,
    pub action: String,
    #[serde(default)]
    pub args: Value,
    pub actor: Actor,
    #[serde(default)]
    pub seen_seq: u64,
    #[serde(default)]
    pub sent_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Actor {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    /// Protocol 1: the role the server checked the command against.
    #[serde(default)]
    pub role: String,
    /// Protocol 2: the actor's permissions when the server forwarded the command.
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
}

impl Actor {
    /// What the actor may do: the permissions (protocol 2), else the role (protocol 1).
    pub fn perms(&self) -> Perms {
        match &self.permissions {
            Some(codes) => Perms::from_codes(codes),
            None => Perms::from_role(&self.role),
        }
    }

    /// A short word for diagnostics and the activity list: the role, else "collaborator".
    pub fn kind(&self) -> &str {
        if self.role.is_empty() {
            "collaborator"
        } else {
            &self.role
        }
    }
}

/// A collaborator connected through the control site (`crew`; names are the owner's to see).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrewMember {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    /// Protocol 1 only (the display role); empty in protocol 2.
    #[serde(default)]
    pub role: String,
    /// Protocol 2: the collaborator's permissions (`None` from a protocol 1 server).
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
    /// Protocol 2: the screens they watch the picture of.
    #[serde(default)]
    pub watching: Vec<Watching>,
    #[serde(default, alias = "since_ms")]
    pub since_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watching {
    #[serde(default)]
    pub screen: String,
    /// `program` | `preview`.
    #[serde(default)]
    pub kind: String,
}

/// What a close code / fatal error means for the app (§10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// The run ended: "Remote control ended" + Start again; never reconnect by itself.
    Ended,
    /// Stop: a problem the user has to fix (sign in, plan, update, clock, moved, deleted).
    Stop,
    /// Reconnect with backoff.
    Reconnect,
    /// Reconnect after the server's `retry_after_s`.
    ReconnectAfter,
}

/// §10's table, by close code or error code.
pub fn close_action(code: &str) -> CloseAction {
    match code {
        "ended" => CloseAction::Ended,
        "unsupported_protocol"
        | "unauthorized"
        | "entitlement_required"
        | "replaced"
        | "show_deleted"
        | "clock_skew" => CloseAction::Stop,
        "rate_limited" | "server_busy" => CloseAction::ReconnectAfter,
        _ => CloseAction::Reconnect,
    }
}

/// The protocol's name for a WebSocket close code (§10).
pub fn close_code_name(code: u16) -> &'static str {
    match code {
        1000 => "ended",
        1001 => "server_restart",
        1009 => "too_large",
        4000 => "bad_message",
        4001 => "unsupported_protocol",
        4401 => "unauthorized",
        4403 => "entitlement_required",
        4408 => "timeout",
        4409 => "replaced",
        4410 => "show_deleted",
        4412 => "clock_skew",
        4413 => "replayed_request",
        4429 => "rate_limited",
        4503 => "server_busy",
        _ => "closed",
    }
}

/// What the app shows for a stopping code (no message from the server, or one §10 says not to
/// show). `entitlement_required` and `replaced` show the server's `message` when it has one.
pub fn stop_message(code: &str, server_message: &str, machine: Option<&str>) -> String {
    let server = server_message.trim();
    match code {
        "unauthorized" => "Sign in again to use remote control.".into(),
        "entitlement_required" if !server.is_empty() => clip(server, MESSAGE_MAX),
        "entitlement_required" => "Your plan doesn't include remote control.".into(),
        "unsupported_protocol" => "Update Kinetra to use remote control.".into(),
        "replaced" => match machine {
            Some(m) => format!("Remote control moved to {}.", clip(m, 60)),
            None if !server.is_empty() => clip(server, MESSAGE_MAX),
            None => "Remote control moved to another computer.".into(),
        },
        "show_deleted" => "This show was deleted on the control site.".into(),
        "clock_skew" => "Your computer's date or time is wrong.".into(),
        _ => "Remote control stopped.".into(),
    }
}

/// At most `max` characters.
pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_serialize_as_the_protocol_shows_them() {
        let m = Out::Diff {
            seq: 43,
            cause: Some("c1".into()),
            ops: vec![
                Op::Put {
                    path: "/layers/l2/on_air".into(),
                    value: json!(true),
                },
                Op::Del {
                    path: "/layers/l9".into(),
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"t":"diff","seq":43,"cause":"c1","ops":[
                {"op":"put","path":"/layers/l2/on_air","value":true},
                {"op":"del","path":"/layers/l9"}]})
        );
        let r = Out::Result(CmdResult {
            corr: "x".into(),
            ok: false,
            code: Some("stale".into()),
            message: Some("Changed on the show machine.".into()),
            details: Some(json!({"fields":["name"]})),
            received_ms: None,
            done_ms: None,
        });
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"t":"result","corr":"x","ok":false,"code":"stale",
                "message":"Changed on the show machine.","details":{"fields":["name"]}})
        );
        let e = Out::End {
            reason: EndReason::ProbeDone,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"t":"end","reason":"probe_done"})
        );
    }

    #[test]
    fn unknown_server_messages_and_fields_are_ignored() {
        let m: In = serde_json::from_str(r#"{"t":"news","x":1}"#).unwrap();
        assert_eq!(m, In::Unknown);
        let w: In = serde_json::from_str(
            r#"{"t":"welcome","protocol":1,"mode":"probe","server_version":"s","server_time_ms":5,
                "heartbeat_s":20,"limits":{"snapshot_bytes":10,"extra":1},"more":true}"#,
        )
        .unwrap();
        match w {
            In::Welcome { mode, limits, .. } => {
                assert_eq!(mode.as_deref(), Some("probe"));
                assert_eq!(limits.snapshot_bytes, 10);
                assert_eq!(limits.diff_bytes, Limits::default().diff_bytes);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn close_codes_map_to_the_table() {
        assert_eq!(close_action(close_code_name(1000)), CloseAction::Ended);
        assert_eq!(close_action(close_code_name(1001)), CloseAction::Reconnect);
        for c in [4001, 4401, 4403, 4409, 4410, 4412] {
            assert_eq!(close_action(close_code_name(c)), CloseAction::Stop, "{c}");
        }
        for c in [4429, 4503] {
            assert_eq!(
                close_action(close_code_name(c)),
                CloseAction::ReconnectAfter
            );
        }
        for c in [1006, 1009, 4000, 4408, 4413] {
            assert_eq!(
                close_action(close_code_name(c)),
                CloseAction::Reconnect,
                "{c}"
            );
        }
    }
}
