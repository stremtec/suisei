//! Shadow WAL — crash-recovery journal for unsaved buffers.
//!
//! D0 from SUISEI-ARCHITECTURE-PLAN: the engine periodically snapshots dirty
//! buffers to `~/.xei/journal/`. On `kill -9` or crash, the next launch finds
//! the journal entries and offers recovery.
//!
//! Flush policy: 250 ms debounce OR 4 KiB accumulated edits since last flush.
//! On explicit save (`:w` / ⌘S) the journal entry is deleted (file is durable).
//!
//! WAL entry format (one file per dirty buffer):
//! ```text
//! SUISEI-WAL v1
//! path: /absolute/path/to/file.rs
//! cursor_row: 42
//! cursor_col: 7
//! scroll: 30
//! timestamp: 1721700000
//! ---
//! <full buffer text>
//! ```

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// Minimum interval between journal flushes (debounce).
const FLUSH_INTERVAL_MS: u128 = 250;
/// If this many bytes of edits accumulate, flush immediately.
const FLUSH_SIZE_THRESHOLD: usize = 4096;

/// One thing to do to the journal directory.
enum WalJob {
    Write { wal: PathBuf, header: String, text: String },
    Remove { wal: PathBuf },
}

/// The writer thread's queue.
///
/// The flush used to happen on the tick: build the whole document, copy it
/// again into a `format!`, create a temp file, write it, **fsync**, rename.
/// Measured 13.7 ms at 10 MiB and 34.3 ms at 30 MiB, against a frame budget of
/// 8.3 ms — every 250 ms, for as long as the buffer stayed dirty. An fsync is
/// a durability barrier; there is no version of it that is fast enough to do
/// between two frames, so it has to happen somewhere else.
///
/// One thread, so writes to one file cannot overlap. Jobs are keyed by WAL
/// path and a new job for a path REPLACES the pending one, which makes the
/// queue's memory one snapshot per dirty file rather than one per flush — the
/// same argument the syntax lane makes about a superseded parse, and the same
/// reason: an older snapshot of a file that has a newer one is not work
/// waiting to be done, it is work that no longer needs doing.
///
/// A delete supersedes a queued write and vice versa, so saving a file cannot
/// leave a stale WAL behind a flush that had not landed yet.
struct WalQueue {
    state: Mutex<WalState>,
    /// Work arrived, or the queue closed.
    ready: Condvar,
    /// The writer has nothing left — what `drain` waits on.
    idle: Condvar,
}

struct WalState {
    order: VecDeque<PathBuf>,
    jobs: HashMap<PathBuf, WalJob>,
    /// A job is being performed right now. `drain` has to wait for it too:
    /// an empty queue with a write in flight is not a written file.
    working: bool,
    closed: bool,
}

impl WalQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(WalState {
                order: VecDeque::new(),
                jobs: HashMap::new(),
                working: false,
                closed: false,
            }),
            ready: Condvar::new(),
            idle: Condvar::new(),
        }
    }

    fn push(&self, key: PathBuf, job: WalJob) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed {
            return;
        }
        if st.jobs.insert(key.clone(), job).is_none() {
            st.order.push_back(key);
        }
        drop(st);
        self.ready.notify_one();
    }

    /// The next job, blocking until there is one. `None` once closed AND
    /// drained — closing does not throw away work the tick already handed over.
    fn take(&self) -> Option<WalJob> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(key) = st.order.pop_front() {
                st.working = true;
                return st.jobs.remove(&key);
            }
            st.working = false;
            self.idle.notify_all();
            if st.closed {
                return None;
            }
            st = self.ready.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Block until the journal directory reflects everything asked for.
    fn drain(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while st.working || !st.order.is_empty() {
            st = self.idle.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn close(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.ready.notify_all();
    }
}

fn wal_writer(queue: Arc<WalQueue>) {
    while let Some(job) = queue.take() {
        match job {
            WalJob::Remove { wal } => {
                let _ = fs::remove_file(wal);
            }
            WalJob::Write { wal, header, text } => {
                // Atomic: temp → fsync → rename, so a crash mid-write never
                // truncates the WAL that is currently recoverable.
                //
                // Header and body are written separately rather than joined.
                // The old `format!` built a second copy of the whole document
                // to prepend six short lines to it — at 30 MiB that allocation
                // and its free were a measurable part of the cost all by
                // themselves.
                let tmp = wal.with_extension("tmp");
                if let Ok(mut f) = fs::File::create(&tmp) {
                    if f.write_all(header.as_bytes()).is_ok()
                        && f.write_all(text.as_bytes()).is_ok()
                    {
                        let _ = f.sync_all();
                        let _ = fs::rename(&tmp, &wal);
                    }
                }
            }
        }
    }
}

/// One pending recovery entry found on startup.
#[derive(Debug, Clone)]
pub struct RecoveryEntry {
    pub file_path: String,
    pub cursor_row: u32,
    pub cursor_col: u32,
    pub scroll: u32,
    pub timestamp: u64,
    pub text: String,
}

/// Shadow WAL journal — owns the `~/.suisei/journal/` directory.
pub struct Journal {
    wal_dir: PathBuf,
    /// file_path → journal file name (hash-based).
    tracked: HashMap<String, String>,
    /// Instant of last successful flush.
    last_flush: Instant,
    /// Bytes of edits since last flush (approximate: buffer_version delta × 64).
    pending_bytes: usize,
    /// Buffer version at last flush (to detect edits).
    last_version: u64,
    /// Recovery entries found on startup (consumed by the face, then cleared).
    pending_recovery: Vec<RecoveryEntry>,
    /// Where the writes actually happen. See [`WalQueue`].
    queue: Arc<WalQueue>,
    writer: Option<std::thread::JoinHandle<()>>,
}

impl Journal {
    /// Create the journal at the default location (`~/.xei/journal/`),
    /// scanning for existing recovery entries.
    pub fn new() -> Self {
        Self::with_dir(Self::wal_dir())
    }

    /// Create the journal at a custom directory (tests, alternate profiles).
    pub fn with_dir(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        let pending_recovery = Self::scan_recovery(&dir);
        let queue = Arc::new(WalQueue::new());
        let mine = Arc::clone(&queue);
        let writer = std::thread::Builder::new()
            .name("suisei-wal".to_string())
            .spawn(move || wal_writer(mine))
            .ok();
        Self {
            wal_dir: dir,
            tracked: HashMap::new(),
            last_flush: Instant::now(),
            pending_bytes: 0,
            last_version: 0,
            pending_recovery,
            queue,
            writer,
        }
    }

    /// Block until every write and delete asked for has happened.
    ///
    /// For tests, and for anything that needs to look at the directory. Nothing
    /// on the tick path calls this — the whole point is that the tick does not
    /// wait for an fsync.
    pub fn drain(&self) {
        self.queue.drain();
    }

    /// Recovery entries found on startup.
    pub fn pending_recovery(&self) -> &[RecoveryEntry] {
        &self.pending_recovery
    }

    /// Number of pending recovery entries.
    pub fn recovery_count(&self) -> usize {
        self.pending_recovery.len()
    }

    /// Get a specific recovery entry by index.
    pub fn recovery_entry(&self, idx: usize) -> Option<&RecoveryEntry> {
        self.pending_recovery.get(idx)
    }

    /// Discard a recovery entry (user chose not to recover) — deletes the WAL file.
    pub fn discard_recovery(&mut self, idx: usize) {
        if idx < self.pending_recovery.len() {
            let entry = self.pending_recovery.remove(idx);
            let name = Self::hash_name(&entry.file_path);
            let wal_path = self.wal_dir.join(&name);
            let _ = fs::remove_file(wal_path);
        }
    }

    /// Accept a recovery entry — returns the text and deletes the WAL file.
    pub fn accept_recovery(&mut self, idx: usize) -> Option<RecoveryEntry> {
        if idx < self.pending_recovery.len() {
            let entry = self.pending_recovery.remove(idx);
            let name = Self::hash_name(&entry.file_path);
            let wal_path = self.wal_dir.join(&name);
            let _ = fs::remove_file(wal_path);
            Some(entry)
        } else {
            None
        }
    }

    /// Called every tick. Flushes the journal if the buffer is dirty and the
    /// flush policy is satisfied.
    ///
    /// `buffer_text` is a **closure**, not a string, and that is the point: this
    /// runs 20 times a second, but the policy above flushes at most every
    /// 250 ms and only while dirty. Taking the text eagerly meant building the
    /// whole document — a `Vec<String>` join plus its allocation and free — on
    /// every tick of every session, clean or not: 0.24 ms per tick on a
    /// 60,000-line file, for a value thrown away 90% of the time
    /// (`tests/tick_breakdown.rs`).
    ///
    /// - `file_path`: current buffer path (empty = untitled).
    /// - `buffer_text`: builds the full buffer content, called only on a flush.
    /// - `buffer_version`: monotonically increasing edit counter.
    /// - `cursor_row`, `cursor_col`, `scroll`: viewport state to restore.
    /// - `dirty`: true if buffer has unsaved changes.
    pub fn on_tick(
        &mut self,
        file_path: &str,
        buffer_text: impl FnOnce() -> String,
        buffer_version: u64,
        cursor_row: u32,
        cursor_col: u32,
        scroll: u32,
        dirty: bool,
    ) {
        if !dirty || file_path.is_empty() {
            // Nothing to journal (clean buffer or untitled).
            // If it WAS tracked and is now clean, remove the entry.
            if !dirty && !file_path.is_empty() {
                self.on_saved(file_path);
            }
            return;
        }

        // Track edit volume for size-based flush.
        if buffer_version != self.last_version {
            let delta = buffer_version.saturating_sub(self.last_version);
            self.pending_bytes += (delta as usize) * 64; // ~64 bytes per edit op estimate
            self.last_version = buffer_version;
        }

        let elapsed = self.last_flush.elapsed().as_millis();
        let should_flush =
            elapsed >= FLUSH_INTERVAL_MS || self.pending_bytes >= FLUSH_SIZE_THRESHOLD;

        if !should_flush {
            return;
        }

        self.flush(file_path, buffer_text(), cursor_row, cursor_col, scroll);
    }

    /// Called on explicit save (⌘S / :w) — the file is now durable, delete WAL.
    ///
    /// Through the same queue as the writes, and that is not tidiness: a delete
    /// done here while a flush for the same path was still queued would be
    /// undone by that flush landing afterwards, leaving a recovery entry for a
    /// file the user had already saved. In the queue the delete supersedes the
    /// write instead.
    pub fn on_saved(&mut self, file_path: &str) {
        if let Some(name) = self.tracked.remove(file_path) {
            let wal = self.wal_dir.join(&name);
            self.queue.push(wal.clone(), WalJob::Remove { wal });
        }
        self.pending_bytes = 0;
    }

    // ─── Internal ─────────────────────────────────────────────────────────────

    /// Hand a snapshot to the writer. Everything expensive happens over there.
    ///
    /// The timestamp is taken HERE, when the snapshot was, not when it reaches
    /// the disk — it is what orders the recovery list, and ordering by when a
    /// queue got round to a file would be ordering by nothing.
    fn flush(
        &mut self,
        file_path: &str,
        buffer_text: String,
        cursor_row: u32,
        cursor_col: u32,
        scroll: u32,
    ) {
        let name = Self::hash_name(file_path);
        let wal = self.wal_dir.join(&name);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let header = format!(
            "SUISEI-WAL v1\npath: {file_path}\ncursor_row: {cursor_row}\n\
             cursor_col: {cursor_col}\nscroll: {scroll}\ntimestamp: {timestamp}\n---\n"
        );

        self.queue.push(
            wal.clone(),
            WalJob::Write {
                wal,
                header,
                text: buffer_text,
            },
        );

        self.tracked.insert(file_path.to_string(), name);
        self.last_flush = Instant::now();
        self.pending_bytes = 0;
    }

    fn wal_dir() -> PathBuf {
        // Suisei-owned, NOT `~/.xei`: the standalone app must not share (and
        // clobber, since names are a path hash) the xei TUI's recovery journal.
        // The rest of the forked state (session/undo/breakpoints) still lives in
        // `~/.xei` — migrating that is a separate independence patch.
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".suisei").join("journal")
    }

    /// Deterministic file name from path (FNV-1a hash → hex).
    fn hash_name(path: &str) -> String {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in path.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{:016x}.wal", hash)
    }

    /// Scan the WAL directory for valid recovery entries.
    fn scan_recovery(wal_dir: &Path) -> Vec<RecoveryEntry> {
        let mut entries = Vec::new();
        let Ok(dir) = fs::read_dir(wal_dir) else {
            return entries;
        };
        for item in dir.flatten() {
            let path = item.path();
            if path.extension().map(|e| e == "wal").unwrap_or(false) {
                if let Some(entry) = Self::parse_wal(&path) {
                    entries.push(entry);
                }
            }
        }
        // Most recent first.
        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        entries
    }

    fn parse_wal(path: &Path) -> Option<RecoveryEntry> {
        let content = fs::read_to_string(path).ok()?;
        let mut lines = content.lines();

        // Header: "SUISEI-WAL v1"
        let magic = lines.next()?;
        if !magic.starts_with("SUISEI-WAL") {
            return None;
        }

        let mut file_path = String::new();
        let mut cursor_row = 0u32;
        let mut cursor_col = 0u32;
        let mut scroll = 0u32;
        let mut timestamp = 0u64;

        // Metadata lines until "---"
        for line in lines.by_ref() {
            if line == "---" {
                break;
            }
            if let Some(v) = line.strip_prefix("path: ") {
                file_path = v.to_string();
            } else if let Some(v) = line.strip_prefix("cursor_row: ") {
                cursor_row = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("cursor_col: ") {
                cursor_col = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("scroll: ") {
                scroll = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("timestamp: ") {
                timestamp = v.parse().unwrap_or(0);
            }
        }

        if file_path.is_empty() {
            return None;
        }

        // Rest is buffer text.
        let text: String = lines.collect::<Vec<_>>().join("\n");

        Some(RecoveryEntry {
            file_path,
            cursor_row,
            cursor_col,
            scroll,
            timestamp,
            text,
        })
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        // Closing drains rather than abandoning: a snapshot the tick already
        // handed over is a snapshot the user is entitled to on the next
        // launch, and quitting is exactly when the buffer might still be
        // dirty.
        self.queue.close();
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
    }
}
