//! Test connection (`docs/CONTROL_PROTOCOL.md` §4a): with remote control off, a **probe**
//! (`?mode=probe`: `hello` → `welcome` → one `ping` → `end {probe_done}` → close; never
//! `register`); with it on, a ping on the live connection (no probe: it would count against the
//! probe limit). Each outcome maps to the §4a message, plus the outcomes §4a doesn't list.

use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;

use super::conn::{self, close, connect, next, send, ConnectError, Got};
use super::diag::ProbeRecord;
use super::ids;
use super::proto::{self, EndReason, In, Out, PROTOCOL};
use super::Ctl;
use crate::core::fsx::now_epoch_ms;
use crate::core::state::AppState;

/// The probe's cap from the upgrade (§4a: the server closes at 15 s).
pub const PROBE_CAP: Duration = Duration::from_secs(15);

/// What Test connection shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    /// `ok`, a protocol / HTTP code (`unauthorized`, `rate_limited`…), `signed_out`, `offline`.
    pub code: String,
    /// What the app shows ("Connected", "Sign in again"…).
    pub message: String,
    /// The error in plain words for "Can't reach Kinetra cloud".
    pub detail: Option<String>,
    pub server_version: Option<String>,
    pub connect_ms: Option<u64>,
    pub welcome_ms: Option<u64>,
    pub rtt_ms: Option<u64>,
    /// Rate limited / busy: the button waits this long.
    pub retry_after_s: Option<u64>,
    /// The probe ran on the live connection (remote control on).
    pub live: bool,
}

fn fail(code: &str, message: impl Into<String>) -> TestResult {
    TestResult {
        ok: false,
        code: code.to_string(),
        message: message.into(),
        ..Default::default()
    }
}

/// §4a's table for an error code (fatal handshake errors and close reasons).
fn for_code(code: &str, server_message: &str) -> TestResult {
    match code {
        "unauthorized" => fail(code, "Sign in again"),
        "entitlement_required" => {
            let m = server_message.trim();
            fail(
                code,
                if m.is_empty() {
                    "Your plan doesn't include remote control.".to_string()
                } else {
                    proto::clip(m, proto::MESSAGE_MAX)
                },
            )
        }
        "clock_skew" => fail(code, "Your computer's date or time is wrong"),
        "unsupported_protocol" => fail(code, "Update Kinetra"),
        "timeout" | "closed" => cant_reach(code, "The connection timed out"),
        "server_busy" => fail(code, "Kinetra cloud is busy, try again later"),
        // replayed_request, bad_message, bad_request: our bug.
        _ => fail(code, "Test failed"),
    }
}

fn cant_reach(code: &str, detail: &str) -> TestResult {
    TestResult {
        detail: Some(detail.to_string()),
        ..fail(code, "Can't reach Kinetra cloud")
    }
}

fn record(st: &AppState, r: &TestResult) {
    st.remote().diag().probe = Some(ProbeRecord {
        at: now_epoch_ms(),
        outcome: r.code.clone(),
        connect_ms: r.connect_ms,
        welcome_ms: r.welcome_ms,
        rtt_ms: r.rtt_ms,
    });
}

/// With remote control live: a ping round trip on the connection.
pub(super) async fn ping_live(st: &AppState, ctl: UnboundedSender<Ctl>) -> TestResult {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let r = if ctl.send(Ctl::Ping(tx)).is_err() {
        cant_reach("offline", "Remote control isn't connected")
    } else {
        match tokio::time::timeout(PROBE_CAP, rx).await {
            Ok(Ok(Ok(rtt))) => TestResult {
                ok: true,
                code: "ok".into(),
                message: "Connected".into(),
                rtt_ms: Some(rtt),
                server_version: st.remote_view().server_version,
                live: true,
                ..Default::default()
            },
            Ok(Ok(Err(why))) => cant_reach("offline", &why),
            _ => cant_reach("timeout", "No answer from Kinetra cloud"),
        }
    };
    record(st, &r);
    r
}

/// The probe (remote control off).
pub(super) async fn probe(st: &AppState) -> TestResult {
    let r = match tokio::time::timeout(PROBE_CAP, run(st)).await {
        Ok(r) => r,
        Err(_) => cant_reach("timeout", "The connection timed out"),
    };
    record(st, &r);
    r
}

async fn run(st: &AppState) -> TestResult {
    // Signed out: nothing is sent.
    let creds = match st.remote_credentials() {
        Ok(c) => c,
        Err(_) => return fail("signed_out", "Sign in to use remote control"),
    };
    let t0 = Instant::now();
    let (mut tx, mut rx) = match connect(&creds.base, &creds.session, true, PROBE_CAP).await {
        Ok(p) => p,
        Err(ConnectError::Http {
            status,
            code,
            retry_after,
        }) => {
            let mut r = match (status, code.as_deref()) {
                (401, _) => fail("unauthorized", "Sign in again"),
                (429, _) => {
                    let n = retry_after.map_or(60, |d| d.as_secs());
                    fail(
                        "rate_limited",
                        format!("Tested too often, try again in {n} s"),
                    )
                }
                (503, _) => {
                    let n = retry_after.map_or(30, |d| d.as_secs());
                    fail(
                        "server_busy",
                        format!("Kinetra cloud is busy, try again in {n} s"),
                    )
                }
                (_, Some(c)) => for_code(c, ""),
                _ => fail(&format!("http_{status}"), "Test failed"),
            };
            r.retry_after_s = retry_after.map(|d| d.as_secs());
            return r;
        }
        Err(ConnectError::Net(why)) => return cant_reach("offline", &why),
    };
    let connect_ms = t0.elapsed().as_millis() as u64;
    let weak = st.downgrade();
    let t = conn::timing();
    let hello = Out::Hello {
        protocol: PROTOCOL,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        machine: creds.machine.clone(),
        license_token: creds.license_token.clone(),
        nonce: ids::nonce(),
        timestamp: creds.now,
    };
    let mut result = TestResult {
        connect_ms: Some(connect_ms),
        ..Default::default()
    };
    if !send(&mut tx, &weak, &hello, t.send).await {
        return TestResult {
            connect_ms: Some(connect_ms),
            ..cant_reach("offline", "The connection dropped")
        };
    }
    let mut fatal: Option<(String, String)> = None;
    let mut corr = String::new();
    let mut ping_sent = 0u64;
    loop {
        match next(&mut rx).await {
            Got::Closed { code, reason } => {
                let code_s = match &fatal {
                    Some((c, _)) => c.clone(),
                    None if !reason.is_empty() => reason,
                    None if code == 1006 => {
                        return TestResult {
                            connect_ms: Some(connect_ms),
                            ..cant_reach("offline", "The connection dropped")
                        }
                    }
                    None => proto::close_code_name(code).to_string(),
                };
                let msg = fatal.map(|(_, m)| m).unwrap_or_default();
                let mut r = for_code(&code_s, &msg);
                r.connect_ms = Some(connect_ms);
                r.welcome_ms = result.welcome_ms;
                r.server_version = result.server_version;
                return r;
            }
            Got::Msg(
                In::Welcome {
                    mode,
                    server_version,
                    ..
                },
                _,
            ) => {
                result.welcome_ms = Some(t0.elapsed().as_millis() as u64);
                result.server_version = Some(server_version);
                if mode.as_deref() != Some("probe") {
                    // A run connection by mistake: leave at once, register nothing.
                    close(&mut tx).await;
                    return TestResult {
                        connect_ms: Some(connect_ms),
                        welcome_ms: result.welcome_ms,
                        ..fail("bad_mode", "Test failed")
                    };
                }
                corr = ids::uuid_v7();
                ping_sent = now_epoch_ms();
                let ping = Out::Ping {
                    corr: corr.clone(),
                    sent_ms: ping_sent,
                };
                if !send(&mut tx, &weak, &ping, t.send).await {
                    return cant_reach("offline", "The connection dropped");
                }
            }
            Got::Msg(
                In::Pong {
                    corr: c,
                    received_ms,
                    sent_ms,
                    ..
                },
                _,
            ) if c == corr && !corr.is_empty() => {
                let now = now_epoch_ms() as i64;
                let rtt = ((now - ping_sent as i64) - (sent_ms as i64 - received_ms as i64)).max(0);
                result.rtt_ms = Some(rtt as u64);
                let _ = send(
                    &mut tx,
                    &weak,
                    &Out::End {
                        reason: EndReason::ProbeDone,
                    },
                    t.send,
                )
                .await;
                // The server closes with 1000; wait briefly, then close ourselves.
                let _ = tokio::time::timeout(t.end_wait, async {
                    while !matches!(next(&mut rx).await, Got::Closed { .. }) {}
                })
                .await;
                close(&mut tx).await;
                return TestResult {
                    ok: true,
                    code: "ok".into(),
                    message: "Connected".into(),
                    ..result
                };
            }
            Got::Msg(
                In::Error {
                    code,
                    message,
                    fatal: true,
                    ..
                },
                _,
            ) => {
                fatal = Some((code, message));
            }
            Got::Msg(In::Error { code, .. }, _) => {
                close(&mut tx).await;
                return for_code(&code, "");
            }
            Got::Msg(In::Ping { corr, sent_ms }, _) => {
                let received_ms = now_epoch_ms();
                let pong = Out::Pong {
                    corr,
                    ping_sent_ms: sent_ms,
                    received_ms,
                    sent_ms: now_epoch_ms(),
                };
                let _ = send(&mut tx, &weak, &pong, t.send).await;
            }
            Got::Msg(..) => {}
        }
    }
}
