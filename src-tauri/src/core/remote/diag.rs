//! Remote control diagnostics: counters per message type, ring buffers of message records,
//! refusals, reconnects and errors, round trips and the last Test connection. In memory only;
//! nothing at all while remote control has never run. The export ([`Diag::export`]) holds IDs,
//! types, sizes, timings and codes only: never field values, state, names, tokens, nonces, the
//! machine fingerprint, account IDs, IP addresses or paths.

use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;
use serde_json::{json, Value};

use crate::core::fsx::now_epoch_ms;

pub const MESSAGES_CAP: usize = 500;
pub const REFUSED_CAP: usize = 50;
pub const RECONNECTS_CAP: usize = 20;
pub const ERRORS_CAP: usize = 50;
/// Round trips kept for the median / max (5 minutes of 20 s pings, plus Test pings).
pub const RTT_WINDOW_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsgRecord {
    pub at: u64,
    pub dir: Dir,
    pub t: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corr: Option<String>,
    pub bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// A result: received → done, in ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub at: u64,
    pub corr: String,
    pub action: String,
    pub role: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cue_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reconnect {
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_code: Option<u16>,
    pub reason: String,
    pub backoff_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorRecord {
    pub at: u64,
    pub code: String,
    pub fatal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corr: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counter {
    pub count: u64,
    pub bytes: u64,
    pub last_at: u64,
}

/// The last Test connection: outcome code (`ok` or the failure's code) and step times.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRecord {
    pub at: u64,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Diag {
    pub server_version: Option<String>,
    /// The protocol the server picked (1 or 2).
    pub protocol: Option<u32>,
    pub run_id: Option<String>,
    pub show_id: Option<String>,
    pub limits: Option<Value>,
    pub counters_in: BTreeMap<&'static str, Counter>,
    pub counters_out: BTreeMap<&'static str, Counter>,
    pub messages: VecDeque<MsgRecord>,
    pub refused: VecDeque<Refusal>,
    pub reconnects: VecDeque<Reconnect>,
    pub errors: VecDeque<ErrorRecord>,
    /// (at, round trip ms).
    pub rtts: VecDeque<(u64, u64)>,
    pub clock_offset_ms: Option<i64>,
    pub connects: u64,
    pub snapshots: u64,
    pub resyncs: u64,
    pub last_resync: Option<(String, String)>,
    pub last_snapshot: Option<(u64, usize)>,
    pub commands: u64,
    pub accepted: u64,
    pub last_close: Option<(u16, String)>,
    pub probe: Option<ProbeRecord>,
    /// Received → done of accepted commands (ms), for the median / max.
    pub result_ms: VecDeque<u64>,
}

fn push<T>(q: &mut VecDeque<T>, cap: usize, v: T) {
    if q.len() == cap {
        q.pop_front();
    }
    q.push_back(v);
}

fn median(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

impl Diag {
    pub fn message(&mut self, rec: MsgRecord) {
        let c = match rec.dir {
            Dir::In => self.counters_in.entry(rec.t).or_default(),
            Dir::Out => self.counters_out.entry(rec.t).or_default(),
        };
        c.count += 1;
        c.bytes += rec.bytes as u64;
        c.last_at = rec.at;
        push(&mut self.messages, MESSAGES_CAP, rec);
    }

    pub fn refusal(&mut self, r: Refusal) {
        push(&mut self.refused, REFUSED_CAP, r);
    }

    pub fn reconnect(&mut self, r: Reconnect) {
        push(&mut self.reconnects, RECONNECTS_CAP, r);
    }

    pub fn error(&mut self, code: &str, fatal: bool, corr: Option<String>) {
        push(
            &mut self.errors,
            ERRORS_CAP,
            ErrorRecord {
                at: now_epoch_ms(),
                code: code.to_string(),
                fatal,
                corr,
            },
        );
    }

    pub fn rtt(&mut self, ms: u64, offset_ms: i64) {
        let now = now_epoch_ms();
        while self
            .rtts
            .front()
            .is_some_and(|(at, _)| now.saturating_sub(*at) > RTT_WINDOW_MS)
        {
            self.rtts.pop_front();
        }
        push(&mut self.rtts, 64, (now, ms));
        self.clock_offset_ms = Some(offset_ms);
    }

    pub fn result_time(&mut self, ms: u64) {
        push(&mut self.result_ms, 200, ms);
    }

    /// Last, median and max round trip over the window.
    pub fn rtt_summary(&self) -> Value {
        let v: Vec<u64> = self.rtts.iter().map(|(_, ms)| *ms).collect();
        json!({
            "last": v.last(),
            "median": median(v.clone()),
            "max": v.iter().max(),
        })
    }

    /// The export (see the module docs and the plan's "Export diagnostics").
    pub fn export(&self, state: &str, app_version: &str, os: &str) -> Value {
        let counters: BTreeMap<&str, Value> = self
            .counters_in
            .keys()
            .chain(self.counters_out.keys())
            .map(|t| {
                let i = self.counters_in.get(t).cloned().unwrap_or_default();
                let o = self.counters_out.get(t).cloned().unwrap_or_default();
                (
                    *t,
                    json!({"in": i.count, "inBytes": i.bytes, "out": o.count, "outBytes": o.bytes}),
                )
            })
            .collect();
        json!({
            "kind": "kinetra/remote-diagnostics",
            "schemaVersion": 1,
            "exportedAt": now_epoch_ms(),
            "appVersion": app_version,
            "os": os,
            "protocol": self.protocol.unwrap_or(super::proto::PROTOCOL),
            "serverVersion": self.server_version,
            "state": state,
            "runId": self.run_id,
            "showId": self.show_id,
            "limits": self.limits,
            "rtt": self.rtt_summary(),
            "clockOffsetMs": self.clock_offset_ms,
            "connects": self.connects,
            "commands": {"received": self.commands, "accepted": self.accepted,
                "resultMsMedian": median(self.result_ms.iter().copied().collect()),
                "resultMsMax": self.result_ms.iter().max()},
            "counters": counters,
            "snapshot": {
                "seq": self.last_snapshot.map(|(s, _)| s),
                "bytes": self.last_snapshot.map(|(_, b)| b),
                "sent": self.snapshots,
                "resyncs": self.resyncs,
                "lastResync": self.last_resync.as_ref().map(|(r, c)| json!({"reason": r, "corr": c})),
            },
            "lastClose": self.last_close.as_ref().map(|(c, r)| json!({"code": c, "reason": r})),
            "reconnects": self.reconnects,
            "messages": self.messages,
            "refused": self.refused,
            "errors": self.errors,
            "probe": self.probe,
        })
    }
}
