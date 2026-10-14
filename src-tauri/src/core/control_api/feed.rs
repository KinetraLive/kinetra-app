//! The state feed: ONE tokio task while at least one client is attached (none otherwise). It
//! builds the tree on the blocking pool when woken (`Notify`), diffs it against the last one and
//! publishes each diff serialized once (`Arc<str>`, shared by every client, like the renderer
//! hub). The first change after a quiet period goes out at once, then at most one diff per
//! [`DIFF_GAP`]. A diff over [`DIFF_OPS_MAX`] ops becomes a new snapshot instead.
//!
//! The current tree and its `seq` sit in a `watch` channel, updated before each diff goes out.
//! Clients ([`Attached`]) subscribe to the frames before reading it, skip diffs it already
//! covers, and get a snapshot of it when they lag (a slow client never holds others back and
//! never queues without bound).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, watch, Notify};
use tokio::time::Instant;

use crate::core::remote::proto::DIFF_OPS_MAX;
use crate::core::remote::state::{self, Model};
use crate::core::state::WeakState;

/// At most one diff per this (the first after a quiet period goes at once).
pub const DIFF_GAP: Duration = Duration::from_millis(50);

/// Frames a client may fall behind before it gets a snapshot instead.
pub const FRAMES_CAP: usize = 64;

/// The current tree and the `seq` it is at (after the last snapshot or diff).
#[derive(Debug)]
pub struct Snap {
    pub seq: u64,
    pub tree: Arc<Value>,
}

impl Snap {
    /// `{"t":"snapshot","seq":…,"state":…}`.
    pub fn text(&self) -> String {
        json!({"t": "snapshot", "seq": self.seq, "state": &*self.tree}).to_string()
    }
}

/// What the feed publishes.
#[derive(Debug)]
pub enum Frame {
    /// A diff, serialized once.
    Diff { seq: u64, text: Arc<str> },
    /// A new snapshot replaced the tree (read it from the snapshot channel).
    Snapshot,
}

/// The channels of a running feed (kept by `ControlApi` while it runs).
pub(super) struct FeedHandle {
    pub gen: u64,
    pub frames: broadcast::Sender<Arc<Frame>>,
    pub snap: watch::Receiver<Option<Arc<Snap>>>,
}

impl FeedHandle {
    /// Starts the feed task on `rt`.
    pub fn start(
        rt: &tokio::runtime::Handle,
        weak: WeakState,
        dirty: Arc<Notify>,
        gen: u64,
        tasks: Arc<AtomicUsize>,
    ) -> Self {
        let (frames, _) = broadcast::channel(FRAMES_CAP);
        let (snap_tx, snap) = watch::channel(None);
        tasks.fetch_add(1, Ordering::Relaxed);
        let running = Running(tasks);
        let tx = frames.clone();
        rt.spawn(async move {
            let _running = running;
            run(weak, dirty, tx, snap_tx, gen).await;
        });
        Self { gen, frames, snap }
    }
}

/// Counts the task in `ControlApi.tasks` until it ends, however it ends.
struct Running(Arc<AtomicUsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn build(weak: &WeakState) -> Option<Value> {
    let st = weak.upgrade()?;
    tokio::task::spawn_blocking(move || st.control_api_tree())
        .await
        .ok()
}

async fn run(
    weak: WeakState,
    dirty: Arc<Notify>,
    frames: broadcast::Sender<Arc<Frame>>,
    snap_tx: watch::Sender<Option<Arc<Snap>>>,
    gen: u64,
) {
    let mut model = Model::default();
    let snapshot = |model: &mut Model, tree: Value| {
        let seq = model.snapshot(tree.clone());
        snap_tx.send_replace(Some(Arc::new(Snap {
            seq,
            tree: Arc::new(tree),
        })));
    };
    let Some(tree) = build(&weak).await else {
        return;
    };
    snapshot(&mut model, tree);
    let mut last_sent = Instant::now();
    loop {
        dirty.notified().await;
        // Stop once the last client has gone (the detach wakes us).
        match weak.upgrade() {
            Some(st) if !st.control_api_feed_done(gen) => {}
            _ => return,
        }
        // Pace: wakes meanwhile coalesce into this one build (a `Notify` keeps one permit).
        let at = last_sent + DIFF_GAP;
        if at > Instant::now() {
            tokio::time::sleep_until(at).await;
        }
        let Some(tree) = build(&weak).await else {
            return;
        };
        let ops = state::diff(&model.tree, &tree);
        if ops.is_empty() {
            continue;
        }
        let cause = weak.upgrade().and_then(|st| st.control_api_take_cause());
        last_sent = Instant::now();
        if ops.len() > DIFF_OPS_MAX {
            snapshot(&mut model, tree);
            let _ = frames.send(Arc::new(Frame::Snapshot));
            continue;
        }
        let current = Arc::new(tree.clone());
        let seq = model.commit_diff(tree, &ops);
        // The current tree first, then the diff: a client attaching in between reads this tree
        // and skips the diff (same seq); one attaching earlier applies it.
        snap_tx.send_replace(Some(Arc::new(Snap { seq, tree: current })));
        let mut msg = json!({"t": "diff", "seq": seq, "ops": ops});
        if let Some(c) = cause {
            msg["cause"] = json!(c);
        }
        let text: Arc<str> = msg.to_string().into();
        // No receiver (every client just left) is fine: the next wake ends the task.
        let _ = frames.send(Arc::new(Frame::Diff { seq, text }));
    }
}

/// A client's view of the feed. Dropping it detaches the client.
pub struct Attached {
    pub id: u64,
    frames: broadcast::Receiver<Arc<Frame>>,
    snap: watch::Receiver<Option<Arc<Snap>>>,
    /// The `seq` the client has (a snapshot's or a diff's): older diffs are skipped.
    seen: Option<u64>,
    /// A close code the app asks for (disconnected, token changed, API off, project closed).
    pub kick: watch::Receiver<Option<u16>>,
    pub(super) _guard: super::DetachGuard,
}

/// The next message for a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// A diff shared by every client.
    Shared(Arc<str>),
    /// A snapshot made for this client.
    Own(String),
}

impl Out {
    pub fn as_str(&self) -> &str {
        match self {
            Out::Shared(s) => s,
            Out::Own(s) => s,
        }
    }
}

impl Attached {
    pub(super) fn new(
        id: u64,
        handle: &FeedHandle,
        kick: watch::Receiver<Option<u16>>,
        guard: super::DetachGuard,
    ) -> Self {
        Self {
            id,
            // Subscribed before the snapshot is read: no diff can fall in between.
            frames: handle.frames.subscribe(),
            snap: handle.snap.clone(),
            seen: None,
            kick,
            _guard: guard,
        }
    }

    /// The latest snapshot (waits for the feed's first tree); `None` when the feed ended.
    pub async fn snapshot(&mut self) -> Option<Out> {
        let snap = self.snap.wait_for(Option::is_some).await.ok()?.clone()?;
        self.seen = Some(snap.seq);
        Some(Out::Own(snap.text()))
    }

    /// The next message: a diff, or the latest snapshot after a lag or an oversized change.
    /// `None` when the feed ended.
    pub async fn next(&mut self) -> Option<Out> {
        if self.seen.is_none() {
            return self.snapshot().await;
        }
        loop {
            match self.frames.recv().await {
                Ok(f) => match &*f {
                    Frame::Diff { seq, text } => {
                        if self.seen.is_some_and(|s| *seq <= s) {
                            continue;
                        }
                        self.seen = Some(*seq);
                        return Some(Out::Shared(text.clone()));
                    }
                    Frame::Snapshot => {
                        let snap = self.snap.borrow().clone();
                        match snap {
                            Some(s) if self.seen.is_none_or(|seen| s.seq > seen) => {
                                self.seen = Some(s.seq);
                                return Some(Out::Own(s.text()));
                            }
                            _ => continue,
                        }
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Too far behind: the latest snapshot instead of the missed diffs.
                    let snap = self.snap.borrow().clone()?;
                    self.seen = Some(snap.seq);
                    return Some(Out::Own(snap.text()));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// A snapshot of the current tree now (a client's `resync`).
    pub fn resync(&mut self) -> Option<Out> {
        let snap = self.snap.borrow().clone()?;
        self.seen = Some(snap.seq);
        Some(Out::Own(snap.text()))
    }

    /// The current tree (commands check against it).
    pub fn tree(&self) -> Option<Arc<Value>> {
        self.snap.borrow().as_ref().map(|s| s.tree.clone())
    }
}
