//! Background syntax parsing — tree-sitter runs HERE, off the keystroke path
//! (A1-6).
//!
//! The engine ships a text snapshot per buffer version (`try_send` — it never
//! blocks on a slow parse); the worker coalesces a typing burst to its newest
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
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender};

/// Work for the syntax worker. A burst coalesces: every `Parse` but the
/// newest is dropped unparsed; every `Prewarm` is honoured.
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

/// Handle to the worker thread. Dropping it closes the request channel,
/// which is what tells the worker to exit.
pub struct SyntaxWorker {
    tx: Vec<SyncSender<SyntaxRequest>>,
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
        let mut tx = Vec::with_capacity(worker_count);
        let mut threads = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            // Bounded per lane: `try_send` never stalls the keystroke path, and
            // a busy language cannot fill every other language's queue.
            let (lane_tx, lane_rx): (SyncSender<SyntaxRequest>, Receiver<SyntaxRequest>) =
                std::sync::mpsc::sync_channel(4);
            let out = frame_tx.clone();
            let counts = Arc::clone(&cache_counts);
            let thread = std::thread::Builder::new()
                .name(format!("suisei-syntax-{index}"))
                .spawn(move || worker_loop(index, lane_rx, out, counts))
                .expect("syntax worker thread");
            tx.push(lane_tx);
            threads.push(thread);
        }
        Self {
            tx,
            rx: frame_rx,
            threads,
        }
    }

    /// Queue work without ever blocking. False when the channel is full —
    /// the worker already holds older requests, and the caller retries on the
    /// next recompose.
    pub fn request(&self, req: SyntaxRequest) -> bool {
        if self.tx.is_empty() {
            return false;
        }
        match &req {
            // Eagerly compiling every grammar in every lane multiplies memory
            // by the CPU count. One lane owns the boot warm; other lanes become
            // warm naturally as project prewarms are distributed to them.
            SyntaxRequest::WarmGrammars => self.tx[0].try_send(req).is_ok(),
            SyntaxRequest::Parse { path, .. } | SyntaxRequest::Prewarm { path, .. } => {
                let lane = lane_for(path, self.tx.len());
                self.tx[lane].try_send(req).is_ok()
            }
        }
    }

    /// Number of independent parser lanes. Exposed for diagnostics and tests;
    /// one document still stays on exactly one lane for incremental correctness.
    pub fn worker_count(&self) -> usize {
        self.tx.len()
    }

    /// Finished frames, drained with `try_recv` at every recompose and tick.
    pub fn frames(&self) -> &Receiver<SyntaxFrame> {
        &self.rx
    }
}

impl Drop for SyntaxWorker {
    fn drop(&mut self) {
        // Close every lane FIRST, then join: `recv` fails and each worker exits.
        self.tx.clear();
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
    rx: Receiver<SyntaxRequest>,
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
        let first = if to_warm.is_empty() {
            match rx.recv() {
                Ok(req) => Some(req),
                Err(_) => return, // request channel closed — the engine is gone
            }
        } else {
            match rx.try_recv() {
                Ok(req) => Some(req),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        };
        let Some(first) = first else {
            // Idle: spend the turn on the next grammar.
            if let Some(lang) = to_warm.pop() {
                engine.warm_one(lang);
            }
            continue;
        };
        // Coalesce the burst: a keystroke queues one snapshot per edit, and
        // only the newest is worth parsing. Prewarms are all honoured.
        let mut prewarms: Vec<SyntaxRequest> = Vec::new();
        let mut parse: Option<SyntaxRequest> = None;
        let mut warm = false;
        match first {
            r @ SyntaxRequest::Parse { .. } => parse = Some(r),
            r @ SyntaxRequest::Prewarm { .. } => prewarms.push(r),
            SyntaxRequest::WarmGrammars => warm = true,
        }
        while let Ok(more) = rx.try_recv() {
            match more {
                r @ SyntaxRequest::Parse { .. } => parse = Some(r),
                r @ SyntaxRequest::Prewarm { .. } => prewarms.push(r),
                SyntaxRequest::WarmGrammars => warm = true,
            }
        }
        // Queue the warm-up rather than doing it here. A Parse coalesced into
        // this same burst builds the one grammar it needs lazily — which is
        // the only grammar that can make the first paint late — and the other
        // twenty-eight are built on later idle turns.
        if warm && to_warm.is_empty() {
            to_warm = Lang::ALL.to_vec();
        }
        let mut cached = false;
        for req in prewarms {
            if let SyntaxRequest::Prewarm { path, ext, text } = req {
                engine.prewarm(&path, &text, ext.as_deref());
                cached = true;
            }
        }
        if let Some(SyntaxRequest::Parse {
            path,
            ext,
            text,
            version,
            window,
        }) = parse
        {
            // Path-aware: adopts a pre-warmed tree on file switches and parks
            // the outgoing one, exactly like the old in-thread path did.
            engine.parse_path(&path, &text, ext.as_deref(), Some(window.clone()));
            cached = true;
            // The tree is cloned rather than moved: this engine keeps its own
            // for the next incremental reparse.
            let tree = engine.live_tree().map(|(t, _)| t.clone());
            let _ = out.send(SyntaxFrame::Tokens {
                tokens: engine.tokens.clone(),
                active: engine.active,
                path,
                version,
                window,
                tree,
                ext: ext.clone().unwrap_or_default(),
                text,
            });
        }
        if cached {
            cache_counts[index].store(engine.cached_count(), Ordering::Relaxed);
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
