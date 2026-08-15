//! Background syntax parsing — tree-sitter runs HERE, off the keystroke path
//! (A1-6).
//!
//! The engine ships a text snapshot per buffer version (into a slot — it never
//! blocks on a slow parse); the lane coalesces a typing burst to its newest
//! snapshot, parses incrementally through its own [`SyntaxEngine`], and sends
//! highlight tokens back as a [`SyntaxFrame`]. The paint path adopts the
//! newest frame that still matches the live document and keeps painting stale
//! tokens until it lands — shifted by a column for a frame or two, exactly
//! like every async highlighter.
//!
//! The pre-warm cache lives here too: the indexer's trees sit next to the
//! parser that reuses them, and the main thread only ever sees tokens.

use crate::lang::Lang;
use crate::syntax::{HlToken, SyntaxEngine};
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

/// Work for the syntax worker.
///
/// A burst coalesces: every `Parse` but the newest is dropped unparsed. Every
/// `Prewarm` is honoured up to the lane's bound and refused past it — see
/// [`Lane`] for why those two are not the same rule.
pub enum SyntaxRequest {
    /// Parse `text` (a snapshot of the live document) and highlight `window`.
    Parse {
        path: String,
        ext: Option<String>,
        text: String,
        version: u64,
        window: Range<usize>,
    },
    /// Parse and cache a file the user is not looking at (project indexer).
    Prewarm {
        path: String,
        ext: Option<String>,
        text: String,
    },
    /// Build every language's parser + highlight query up front (boot
    /// pipeline), so the first file opened never pays a cold grammar build.
    WarmGrammars,
}

/// A finished unit of syntax work, adopted at the next recompose or tick.
pub enum SyntaxFrame {
    /// Highlight tokens for the snapshot named by `path` + `version`. Applied
    /// only while both still match the live document; anything else is stale
    /// and dropped.
    Tokens {
        path: String,
        version: u64,
        window: Range<usize>,
        tokens: Vec<HlToken>,
        active: bool,
        /// The file's global scope, collected here rather than on the main
        /// thread. Completion needs it and it costs 8.7 ms at 50k lines; the
        /// parse that produces it already happened on this thread.
        globals: Vec<crate::scope::Found>,
        /// The parse itself, handed to the main thread with the tokens.
        ///
        /// Highlighting only ever needed the tokens, so the tree used to stay
        /// on this thread and `apply_frame` set `self.tree = None`. That made
        /// `live_tree()` permanently `None` in the GUI — and scope-aware
        /// completion, which is defined against the live tree, silently
        /// returned nothing for every keystroke. It worked in tests only
        /// because those call `syntax.parse()` directly, which is the TUI path.
        ///
        /// `Tree` is `Send`, and the snapshot text already crosses in the other
        /// direction every keystroke, so carrying both back is the same trade
        /// the request side already makes.
        tree: Option<tree_sitter::Tree>,
        /// Text the tree was parsed from. Byte offsets are meaningless without
        /// it, so it travels with the tree or not at all.
        text: String,
        /// Extension the tree was parsed as, for language lookup.
        ext: String,
    },
    /// The worker's pre-parse cache size — mirrors into the FFI diagnostic.
    Cached { count: usize },
}

/// How many speculative parses one lane will hold.
///
/// The same bound the lane used to have for everything. It stays where it is
/// because it is a memory bound — the indexer hands over whole file texts —
/// and the live parse no longer competes for it.
const PREWARM_DEPTH: usize = 4;

/// What a lane had to say when asked for work.
enum Taken {
    Work(SyntaxRequest),
    /// Nothing queued. Only ever returned to a caller that said not to block.
    Idle,
    /// The engine is gone.
    Closed,
}

/// One lane's pending work — a priority queue, not a queue.
///
/// It was `sync_channel(4)`: four slots shared by the live document and by the
/// project indexer's speculative parses. A queue answers "what arrived first",
/// and that is the wrong question in exactly the case that hurt — the indexer
/// fills a lane, the user types, and `try_send` refuses the one request
/// somebody is waiting on. Measured 1.4 s behind a 2.4 MB prewarm set.
///
/// The live snapshot gets a slot of its own that cannot be refused, and an
/// older snapshot of the same document is overwritten rather than queued: it
/// is not partial work, it is wrong work. The worker already coalesced those
/// away after arrival; this does it before, where a full queue can no longer
/// refuse the newest one.
///
/// Speculative work keeps the bound and keeps being refused when full. That is
/// backpressure doing its job — nobody is waiting on it — and it is only a bug
/// when the live parse shares the bound.
struct Lane {
    parse: Option<SyntaxRequest>,
    prewarm: VecDeque<SyntaxRequest>,
    warm: bool,
    closed: bool,
}

struct LaneQueue {
    work: Mutex<Lane>,
    ready: Condvar,
}

impl LaneQueue {
    fn new() -> Self {
        Self {
            work: Mutex::new(Lane {
                parse: None,
                prewarm: VecDeque::with_capacity(PREWARM_DEPTH),
                warm: false,
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    /// Queue one request. Never blocks on the worker — the lock is held for a
    /// move and released, never across a parse.
    ///
    /// False means the request was not taken, which can now only happen to a
    /// `Prewarm`.
    fn push(&self, req: SyntaxRequest) -> bool {
        let mut lane = self.work.lock().unwrap_or_else(|e| e.into_inner());
        if lane.closed {
            return false;
        }
        let accepted = match req {
            SyntaxRequest::Parse { .. } => {
                lane.parse = Some(req);
                true
            }
            SyntaxRequest::Prewarm { .. } => {
                if lane.prewarm.len() >= PREWARM_DEPTH {
                    false
                } else {
                    lane.prewarm.push_back(req);
                    true
                }
            }
            SyntaxRequest::WarmGrammars => {
                lane.warm = true;
                true
            }
        };
        drop(lane);
        if accepted {
            self.ready.notify_one();
        }
        accepted
    }

    /// One unit of work, live parse first.
    ///
    /// One unit, not the whole batch. Draining every queued prewarm before
    /// looking at the parse slot again would put the live document back behind
    /// the same 2.4 MB of speculative work the slot exists to get it out from
    /// under — the wait would be bounded by the queue instead of by the
    /// channel, which is the same wait. A parse that arrives mid-sweep now
    /// waits for at most the one prewarm already in flight.
    fn take(&self, block: bool) -> Taken {
        let mut lane = self.work.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if lane.closed {
                return Taken::Closed;
            }
            if let Some(req) = lane.parse.take() {
                return Taken::Work(req);
            }
            if let Some(req) = lane.prewarm.pop_front() {
                return Taken::Work(req);
            }
            if lane.warm {
                lane.warm = false;
                return Taken::Work(SyntaxRequest::WarmGrammars);
            }
            if !block {
                return Taken::Idle;
            }
            lane = self.ready.wait(lane).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Tell the worker to exit and wake it if it is waiting.
    fn close(&self) {
        self.work.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.ready.notify_all();
    }
}

/// Handle to the worker threads. Dropping it closes every lane, which is what
/// tells the workers to exit.
pub struct SyntaxWorker {
    lanes: Vec<Arc<LaneQueue>>,
    rx: Receiver<SyntaxFrame>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl SyntaxWorker {
    pub fn start() -> Self {
        // Tree-sitter itself parses one document serially. Parallelism belongs
        // between documents: project prewarms and independent open buffers can
        // then use otherwise idle cores without violating one buffer's version
        // order. Keep two logical CPUs free for AppKit/Metal and the engine.
        let worker_count = syntax_worker_count();
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        let cache_counts = Arc::new(
            (0..worker_count)
                .map(|_| AtomicUsize::new(0))
                .collect::<Vec<_>>(),
        );
        let mut lanes = Vec::with_capacity(worker_count);
        let mut threads = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            // Bounded per lane: pushing never stalls the keystroke path, and a
            // busy language cannot fill every other language's queue.
            let lane = Arc::new(LaneQueue::new());
            let mine = Arc::clone(&lane);
            let out = frame_tx.clone();
            let counts = Arc::clone(&cache_counts);
            let thread = std::thread::Builder::new()
                .name(format!("suisei-syntax-{index}"))
                .spawn(move || worker_loop(index, mine, out, counts))
                .expect("syntax worker thread");
            lanes.push(lane);
            threads.push(thread);
        }
        Self {
            lanes,
            rx: frame_rx,
            threads,
        }
    }

    /// Queue work without ever blocking.
    ///
    /// A live `Parse` is always accepted — it replaces the older snapshot of
    /// the same document, which nobody wanted parsed anyway. False now means
    /// only that a lane's speculative queue is full, and the caller may retry.
    pub fn request(&self, req: SyntaxRequest) -> bool {
        if self.lanes.is_empty() {
            return false;
        }
        match &req {
            // Eagerly compiling every grammar in every lane multiplies memory
            // by the CPU count. One lane owns the boot warm; other lanes become
            // warm naturally as project prewarms are distributed to them.
            SyntaxRequest::WarmGrammars => self.lanes[0].push(req),
            SyntaxRequest::Parse { path, .. } | SyntaxRequest::Prewarm { path, .. } => {
                let lane = lane_for(path, self.lanes.len());
                self.lanes[lane].push(req)
            }
        }
    }

    /// Number of independent parser lanes. Exposed for diagnostics and tests;
    /// one document still stays on exactly one lane for incremental correctness.
    pub fn worker_count(&self) -> usize {
        self.lanes.len()
    }

    /// Finished frames, drained with `try_recv` at every recompose and tick.
    pub fn frames(&self) -> &Receiver<SyntaxFrame> {
        &self.rx
    }
}

impl Drop for SyntaxWorker {
    fn drop(&mut self) {
        // Close every lane FIRST, then join: each worker's next `take` — or the
        // wait it is parked in — answers `Closed` and it returns.
        for lane in &self.lanes {
            lane.close();
        }
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

fn syntax_worker_count() -> usize {
    if let Ok(raw) = std::env::var("SUISEI_SYNTAX_WORKERS")
        && let Ok(requested) = raw.parse::<usize>()
    {
        return requested.clamp(1, 16);
    }
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
        .saturating_sub(2)
        .clamp(1, 8)
}

fn lane_for(path: &str, lanes: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish() as usize % lanes.max(1)
}

fn worker_loop(
    index: usize,
    lane: Arc<LaneQueue>,
    out: Sender<SyntaxFrame>,
    cache_counts: Arc<Vec<AtomicUsize>>,
) {
    let mut engine = SyntaxEngine::new();
    // Grammars still to build, drained ONE per idle turn. Warming all of them
    // in a single call took 780 ms once the table reached 29 languages —
    // Haskell alone is 186 ms — and it ran before the first parse, so the
    // first file opened would have waited the whole time for its colours.
    // Draining it a grammar at a time bounds that wait to one build, and only
    // when nothing else is queued.
    let mut to_warm: Vec<Lang> = Vec::new();
    loop {
        // Only block when there is no warming left to do; otherwise take what
        // is waiting and fall through to build one grammar.
        let req = match lane.take(to_warm.is_empty()) {
            Taken::Closed => return, // the engine is gone
            Taken::Idle => {
                // Idle: spend the turn on the next grammar.
                if let Some(lang) = to_warm.pop() {
                    engine.warm_one(lang);
                }
                continue;
            }
            Taken::Work(req) => req,
        };
        match req {
            // Queue the warm-up rather than doing it here. A Parse that arrives
            // while this is queued builds the one grammar it needs lazily —
            // which is the only grammar that can make the first paint late —
            // and the other twenty-eight are built on later idle turns.
            SyntaxRequest::WarmGrammars => {
                if to_warm.is_empty() {
                    to_warm = Lang::ALL.to_vec();
                }
                continue;
            }
            SyntaxRequest::Prewarm { path, ext, text } => {
                engine.prewarm(&path, &text, ext.as_deref());
            }
            SyntaxRequest::Parse {
                path,
                ext,
                text,
                version,
                window,
            } => {
                // Path-aware: adopts a pre-warmed tree on file switches and
                // parks the outgoing one, exactly like the old in-thread path.
                engine.parse_path(&path, &text, ext.as_deref(), Some(window.clone()));
                // The tree is cloned rather than moved: this engine keeps its
                // own for the next incremental reparse.
                let tree = engine.live_tree().map(|(t, _)| t.clone());
                let globals = match (
                    engine.live_tree(),
                    crate::scope::ScopeLang::from_ext(engine.live_ext()),
                ) {
                    (Some((t, txt)), Some(lang)) => {
                        crate::scope::collect_global_symbols(t, txt, lang)
                    }
                    _ => Vec::new(),
                };
                let _ = out.send(SyntaxFrame::Tokens {
                    tokens: engine.tokens.clone(),
                    active: engine.active,
                    globals,
                    path,
                    version,
                    window,
                    tree,
                    ext: ext.clone().unwrap_or_default(),
                    text,
                });
            }
        }
        // One unit of work per turn now, so this would report the same total
        // over and over during an indexing sweep. Only a change is news.
        let count = engine.cached_count();
        if cache_counts[index].swap(count, Ordering::Relaxed) != count {
            let _ = out.send(SyntaxFrame::Cached {
                count: cache_counts
                    .iter()
                    .map(|count| count.load(Ordering::Relaxed))
                    .sum(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_snapshot_and_answers_with_tokens() {
        let worker = SyntaxWorker::start();
        assert!(worker.worker_count() >= 1);
        assert!(worker.request(SyntaxRequest::Parse {
            path: "/tmp/suisei_worker_test.rs".to_string(),
            ext: Some("rs".to_string()),
            text: "fn main() { let x = 1; }\n".to_string(),
            version: 7,
            window: 0..100,
        }));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut got = None;
        while std::time::Instant::now() < deadline {
            if let Ok(frame) = worker
                .frames()
                .recv_timeout(std::time::Duration::from_millis(20))
            {
                if let SyntaxFrame::Tokens {
                    version,
                    tokens,
                    active,
                    ..
                } = frame
                {
                    got = Some((version, !tokens.is_empty(), active));
                    break;
                }
            }
        }
        assert_eq!(
            got,
            Some((7, true, true)),
            "worker should answer with live tokens"
        );
    }

    #[test]
    fn a_burst_always_parses_the_newest_snapshot() {
        let worker = SyntaxWorker::start();
        let send_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for v in 1..=20u64 {
            // Deliver every version — retrying, exactly like the engine does
            // on the next recompose when the channel is momentarily full.
            while !worker.request(SyntaxRequest::Parse {
                path: "/tmp/suisei_worker_burst.rs".to_string(),
                ext: Some("rs".to_string()),
                text: format!("fn f() {{ let v{v} = {v}; }}\n"),
                version: v,
                window: 0..100,
            }) {
                assert!(
                    std::time::Instant::now() < send_deadline,
                    "channel never drained"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        // Whatever the interleaving with the worker: answers arrive in
        // request order, every answer was requested, and the newest snapshot
        // is parsed last (older snapshots queued during a parse coalesce).
        let mut seen: Vec<u64> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && seen.last() != Some(&20) {
            match worker
                .frames()
                .recv_timeout(std::time::Duration::from_millis(50))
            {
                Ok(SyntaxFrame::Tokens { version, .. }) => seen.push(version),
                Ok(SyntaxFrame::Cached { .. }) => {}
                Err(_) => {}
            }
        }
        assert_eq!(
            seen.last(),
            Some(&20),
            "the newest snapshot must be parsed: {seen:?}"
        );
        assert!(seen.iter().all(|v| (1..=20).contains(v)));
        assert!(
            seen.windows(2).all(|w| w[0] < w[1]),
            "answers arrive in order: {seen:?}"
        );
    }

    /// A live snapshot is never refused, however full the lane is.
    ///
    /// The lane is exercised directly rather than through a running worker:
    /// the property is about the producer side, and a thread that drains
    /// would make "full" a race. Under `sync_channel(4)` the fifth push here
    /// was the one the user was waiting on, and it failed.
    #[test]
    fn a_full_lane_still_takes_the_live_snapshot() {
        let lane = LaneQueue::new();
        for i in 0..PREWARM_DEPTH {
            assert!(
                lane.push(SyntaxRequest::Prewarm {
                    path: format!("/tmp/p{i}.rs"),
                    ext: Some("rs".into()),
                    text: "fn a() {}\n".into(),
                }),
                "prewarm {i} should fit"
            );
        }
        assert!(
            !lane.push(SyntaxRequest::Prewarm {
                path: "/tmp/overflow.rs".into(),
                ext: Some("rs".into()),
                text: "fn a() {}\n".into(),
            }),
            "speculative work keeps its bound"
        );
        assert!(
            lane.push(SyntaxRequest::Parse {
                path: "/tmp/live.rs".into(),
                ext: Some("rs".into()),
                text: "fn live() {}\n".into(),
                version: 1,
                window: 0..100,
            }),
            "the live snapshot has a slot of its own"
        );
        // And it comes out FIRST, ahead of four prewarms already waiting.
        match lane.take(false) {
            Taken::Work(SyntaxRequest::Parse { version, .. }) => assert_eq!(version, 1),
            _ => panic!("the live parse must be served before speculative work"),
        }
    }

    /// An older snapshot of the same document is not partial work.
    #[test]
    fn a_newer_snapshot_replaces_the_one_waiting() {
        let lane = LaneQueue::new();
        for version in 1..=9u64 {
            assert!(lane.push(SyntaxRequest::Parse {
                path: "/tmp/live.rs".into(),
                ext: Some("rs".into()),
                text: format!("fn v{version}() {{}}\n"),
                version,
                window: 0..100,
            }));
        }
        match lane.take(false) {
            Taken::Work(SyntaxRequest::Parse { version, .. }) => assert_eq!(version, 9),
            _ => panic!("expected the newest snapshot"),
        }
        assert!(
            matches!(lane.take(false), Taken::Idle),
            "the older eight were replaced, not queued"
        );
    }

    /// A parse queued mid-sweep waits for one prewarm, not for the sweep.
    ///
    /// This is the shape of the 1.4 s stall: the worker used to drain the whole
    /// channel into a batch and run every prewarm in it before looking again.
    #[test]
    fn a_parse_cuts_into_a_prewarm_sweep() {
        let lane = LaneQueue::new();
        for i in 0..PREWARM_DEPTH {
            lane.push(SyntaxRequest::Prewarm {
                path: format!("/tmp/p{i}.rs"),
                ext: Some("rs".into()),
                text: "fn a() {}\n".into(),
            });
        }
        // The worker takes one prewarm...
        assert!(matches!(
            lane.take(false),
            Taken::Work(SyntaxRequest::Prewarm { .. })
        ));
        // ...and while it is parsing it, the user types.
        lane.push(SyntaxRequest::Parse {
            path: "/tmp/live.rs".into(),
            ext: Some("rs".into()),
            text: "fn live() {}\n".into(),
            version: 3,
            window: 0..100,
        });
        match lane.take(false) {
            Taken::Work(SyntaxRequest::Parse { version, .. }) => assert_eq!(version, 3),
            _ => panic!("the live parse waits for the prewarm in flight, not for the sweep"),
        }
    }

    #[test]
    fn the_same_path_always_uses_the_same_lane() {
        for lanes in 1..=8 {
            let a = lane_for("/tmp/project/src/main.rs", lanes);
            let b = lane_for("/tmp/project/src/main.rs", lanes);
            assert_eq!(a, b);
            assert!(a < lanes);
        }
    }
}
