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

use crate::syntax::{HlToken, SyntaxEngine};
use std::ops::Range;
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
    },
    /// The worker's pre-parse cache size — mirrors into the FFI diagnostic.
    Cached { count: usize },
}

/// Handle to the worker thread. Dropping it closes the request channel,
/// which is what tells the worker to exit.
pub struct SyntaxWorker {
    tx: Option<SyncSender<SyntaxRequest>>,
    rx: Receiver<SyntaxFrame>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SyntaxWorker {
    pub fn start() -> Self {
        // Bounded so a slow worker can never stall a typing run: `try_send`
        // drops instead of blocking the keystroke path, and the worker drains
        // to the newest snapshot anyway.
        let (tx, rx): (SyncSender<SyntaxRequest>, Receiver<SyntaxRequest>) =
            std::sync::mpsc::sync_channel(4);
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("suisei-syntax".into())
            .spawn(move || worker_loop(rx, frame_tx))
            .expect("syntax worker thread");
        Self {
            tx: Some(tx),
            rx: frame_rx,
            thread: Some(thread),
        }
    }

    /// Queue work without ever blocking. False when the channel is full —
    /// the worker already holds older requests, and the caller retries on the
    /// next recompose.
    pub fn request(&self, req: SyntaxRequest) -> bool {
        self.tx
            .as_ref()
            .map(|tx| tx.try_send(req).is_ok())
            .unwrap_or(false)
    }

    /// Finished frames, drained with `try_recv` at every recompose and tick.
    pub fn frames(&self) -> &Receiver<SyntaxFrame> {
        &self.rx
    }
}

impl Drop for SyntaxWorker {
    fn drop(&mut self) {
        // Close the channel FIRST, then join: `recv` fails and the worker
        // exits. Without the `take`, the sender would outlive the join and
        // deadlock it.
        self.tx.take();
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

fn worker_loop(rx: Receiver<SyntaxRequest>, out: Sender<SyntaxFrame>) {
    let mut engine = SyntaxEngine::new();
    loop {
        let first = match rx.recv() {
            Ok(req) => req,
            Err(_) => return, // request channel closed — the engine is gone
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
        // Warm before parsing: a Parse coalesced into the same burst then finds
        // its grammar already built instead of paying for it inline.
        if warm {
            engine.warm_all();
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
            let _ = out.send(SyntaxFrame::Tokens {
                tokens: engine.tokens.clone(),
                active: engine.active,
                path,
                version,
                window,
            });
        }
        if cached {
            let _ = out.send(SyntaxFrame::Cached {
                count: engine.cached_count(),
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
}
