//! The NDI® outputs' host (Windows): one frame source and one NDI sender per planned NDI output
//! (`core::outputs::plan_ndi`), started and stopped by reconciles on the blocking pool.
//!
//! A changed plan (screen, size, rate, alpha, port, name, groups) restarts that output's source;
//! unchanged ones keep running untouched. The runtime is loaded with the first source and
//! destroyed with the last (`Runtime::acquire`). Receiver and tally changes come from the sender
//! threads and are passed to the core with [`NdiOutputs::new`]'s `notify` (which never blocks).
//!
//! **Crash recovery:** when a capture page's WebView2 process fails, the output is marked crashed
//! and `restart` asks the core for a reconcile; the next `apply` starts it again. At most
//! [`RESTARTS_MAX`] times per [`RESTART_WINDOW`]; after that it stays stopped and reports
//! [`PAGE_CRASHING`] until its plan changes (turned off and on, another screen, rate…), like
//! display output windows.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::core::model::NdiLive;
use crate::core::outputs::{NdiHost, NdiPlan, NDI_MISSING};
use crate::frames::{Adapter, FrameSource, Outlet, Page, SourceConfig};

use super::sender::{NdiSender, OnChange, SendMode, SenderStats};
use super::{NdiError, Runtime};

/// Restarts of one output allowed within [`RESTART_WINDOW`].
pub const RESTARTS_MAX: usize = 3;
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// The failure reported for an output whose page kept crashing.
pub const PAGE_CRASHING: &str = "The NDI output page keeps crashing";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Recent restarts of one output; `true` when one more is allowed at `now` (and records it).
fn allow_restart(history: &mut VecDeque<Instant>, now: Instant) -> bool {
    while history
        .front()
        .is_some_and(|t| now.duration_since(*t) >= RESTART_WINDOW)
    {
        history.pop_front();
    }
    if history.len() >= RESTARTS_MAX {
        return false;
    }
    history.push_back(now);
    true
}

/// One running NDI output. Dropped in field order: the sender stops first, then the source.
struct Running {
    plan: NdiPlan,
    sender: Option<NdiSender>,
    outlet: Arc<Outlet>,
    source: Option<FrameSource>,
    _runtime: Arc<Runtime>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(s) = self.source.take() {
            s.unsubscribe(&self.outlet);
        }
    }
}

pub struct NdiOutputs {
    me: Weak<Self>,
    /// The capture pages' WebView2 user data folder.
    data_dir: PathBuf,
    notify: OnChange,
    /// Asks the core for a reconcile (a capture page failed). Never blocks.
    restart: OnChange,
    /// Held for a whole `apply` (starting a source takes a moment).
    running: Mutex<HashMap<String, Running>>,
    /// The senders' stats for [`NdiHost::live`], which must never wait for `apply`.
    stats: Mutex<HashMap<String, Arc<SenderStats>>>,
    /// Outputs whose page failed since the last `apply`.
    crashed: Mutex<HashSet<String>>,
    /// Restarts per output within the last [`RESTART_WINDOW`].
    restarts: Mutex<HashMap<String, VecDeque<Instant>>>,
    /// Outputs given up on, with the plan they kept crashing with.
    gave_up: Mutex<HashMap<String, NdiPlan>>,
}

impl NdiOutputs {
    pub fn new(data_dir: PathBuf, notify: OnChange, restart: OnChange) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            data_dir,
            notify,
            restart,
            running: Mutex::new(HashMap::new()),
            stats: Mutex::new(HashMap::new()),
            crashed: Mutex::new(HashSet::new()),
            restarts: Mutex::new(HashMap::new()),
            gave_up: Mutex::new(HashMap::new()),
        })
    }

    fn start(&self, plan: &NdiPlan) -> Result<Running, String> {
        let runtime = Runtime::acquire().map_err(|e| match e {
            NdiError::RuntimeMissing => NDI_MISSING.to_string(),
            e => e.to_string(),
        })?;
        let (me, id) = (self.me.clone(), plan.id.clone());
        let on_failed: crate::frames::capture::OnFailed = Arc::new(move || {
            if let Some(me) = me.upgrade() {
                lock(&me.crashed).insert(id.clone());
                (me.restart)();
            }
        });
        let source = FrameSource::start(
            SourceConfig {
                page: Page::Url(plan.url.clone()),
                width: plan.width,
                height: plan.height,
                rate: plan.rate,
                format: plan.format,
                alpha: plan.alpha,
                background: plan.background,
                data_dir: self.data_dir.clone(),
                adapter: Adapter::Primary,
                follow_page: true,
            },
            Some(on_failed),
        )
        .map_err(|e| format!("Couldn't start: {e}"))?;
        let outlet = source.subscribe();
        let sender = NdiSender::start(
            runtime.clone(),
            &plan.name,
            plan.groups.as_deref(),
            outlet.clone(),
            SendMode::Async,
            Some(self.notify.clone()),
        );
        let sender = match sender {
            Ok(s) => s,
            Err(e) => {
                source.unsubscribe(&outlet);
                return Err(e.to_string());
            }
        };
        Ok(Running {
            plan: plan.clone(),
            sender: Some(sender),
            outlet,
            source: Some(source),
            _runtime: runtime,
        })
    }
}

impl NdiHost for NdiOutputs {
    fn apply(&self, plans: &[NdiPlan]) -> HashMap<String, String> {
        let mut running = lock(&self.running);
        // An output given up on stays stopped while its plan is unchanged; any change (or the
        // output turned off) gives it a fresh start.
        {
            let mut gave_up = lock(&self.gave_up);
            gave_up.retain(|_, plan| plans.contains(plan));
            lock(&self.restarts)
                .retain(|id, _| gave_up.contains_key(id) || plans.iter().any(|p| p.id == *id));
        }
        let crashed: HashSet<String> = std::mem::take(&mut *lock(&self.crashed));
        // Stop what is gone, changed or crashed first (a renamed source frees its name).
        let stale: Vec<String> = running
            .iter()
            .filter(|(id, r)| {
                crashed.contains(*id) || !plans.iter().any(|p| p.id == **id && p == &r.plan)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            lock(&self.stats).remove(&id);
            running.remove(&id);
        }
        let mut failures = HashMap::new();
        for p in plans {
            if running.contains_key(&p.id) {
                continue;
            }
            if lock(&self.gave_up).contains_key(&p.id) {
                failures.insert(p.id.clone(), PAGE_CRASHING.to_string());
                continue;
            }
            if crashed.contains(&p.id) {
                let again = allow_restart(
                    lock(&self.restarts).entry(p.id.clone()).or_default(),
                    Instant::now(),
                );
                tracing::warn!(
                    "NDI output {}: capture page failed; {}",
                    p.id,
                    if again { "restarting it" } else { "giving up" }
                );
                if !again {
                    lock(&self.gave_up).insert(p.id.clone(), p.clone());
                    failures.insert(p.id.clone(), PAGE_CRASHING.to_string());
                    continue;
                }
            }
            match self.start(p) {
                Ok(r) => {
                    if let Some(s) = &r.sender {
                        lock(&self.stats).insert(p.id.clone(), s.stats.clone());
                    }
                    tracing::info!(
                        "NDI output {} sending {}×{} at {}",
                        p.id,
                        p.width,
                        p.height,
                        p.rate.label()
                    );
                    running.insert(p.id.clone(), r);
                }
                Err(e) => {
                    failures.insert(p.id.clone(), e);
                }
            }
        }
        failures
    }

    fn live(&self) -> HashMap<String, NdiLive> {
        lock(&self.stats)
            .iter()
            .map(|(id, s)| {
                let t = s.tally();
                (
                    id.clone(),
                    NdiLive {
                        receivers: u32::try_from(
                            s.receivers.load(std::sync::atomic::Ordering::Relaxed),
                        )
                        .unwrap_or(0),
                        program: t.on_program,
                        preview: t.on_preview,
                        dropping: s.dropping.load(std::sync::atomic::Ordering::Relaxed),
                    },
                )
            })
            .collect()
    }

    fn close_all(&self) {
        lock(&self.stats).clear();
        lock(&self.running).clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_restarts_per_minute() {
        let t0 = Instant::now();
        let mut h = VecDeque::new();
        let s = Duration::from_secs;
        assert!(allow_restart(&mut h, t0));
        assert!(allow_restart(&mut h, t0 + s(10)));
        assert!(allow_restart(&mut h, t0 + s(20)));
        assert!(!allow_restart(&mut h, t0 + s(30)), "a 4th within a minute");
        assert!(allow_restart(&mut h, t0 + s(60)));
        assert!(!allow_restart(&mut h, t0 + s(61)));
    }
}
