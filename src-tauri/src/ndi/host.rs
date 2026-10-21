//! The NDI® outputs' host (Windows): one frame source and one NDI sender per planned NDI output
//! (`core::outputs::plan_ndi`), started and stopped by reconciles on the blocking pool.
//!
//! A changed plan (screen, size, rate, alpha, port, name, groups) restarts that output's source;
//! unchanged ones keep running untouched. The runtime is loaded with the first source and
//! destroyed with the last (`Runtime::acquire`). Receiver and tally changes come from the sender
//! threads and are passed to the core with [`NdiOutputs::new`]'s `notify` (which never blocks).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::core::model::NdiLive;
use crate::core::outputs::{NdiHost, NdiPlan, NDI_MISSING};
use crate::frames::{Adapter, FrameSource, Outlet, Page, SourceConfig};

use super::sender::{NdiSender, OnChange, SendMode, SenderStats};
use super::{NdiError, Runtime};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// The capture pages' WebView2 user data folder.
    data_dir: PathBuf,
    notify: OnChange,
    /// Held for a whole `apply` (starting a source takes a moment).
    running: Mutex<HashMap<String, Running>>,
    /// The senders' stats for [`NdiHost::live`], which must never wait for `apply`.
    stats: Mutex<HashMap<String, Arc<SenderStats>>>,
}

impl NdiOutputs {
    pub fn new(data_dir: PathBuf, notify: OnChange) -> Arc<Self> {
        Arc::new(Self {
            data_dir,
            notify,
            running: Mutex::new(HashMap::new()),
            stats: Mutex::new(HashMap::new()),
        })
    }

    fn start(&self, plan: &NdiPlan) -> Result<Running, String> {
        let runtime = Runtime::acquire().map_err(|e| match e {
            NdiError::RuntimeMissing => NDI_MISSING.to_string(),
            e => e.to_string(),
        })?;
        let source = FrameSource::start(SourceConfig {
            page: Page::Url(plan.url.clone()),
            width: plan.width,
            height: plan.height,
            rate: plan.rate,
            format: plan.format,
            alpha: plan.alpha,
            background: plan.background,
            data_dir: self.data_dir.clone(),
            adapter: Adapter::HighPerformance,
        })
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
        // Stop what is gone or changed first (a renamed source frees its name).
        let stale: Vec<String> = running
            .iter()
            .filter(|(id, r)| !plans.iter().any(|p| p.id == **id && p == &r.plan))
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
