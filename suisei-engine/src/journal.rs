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

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Minimum interval between journal flushes (debounce).
const FLUSH_INTERVAL_MS: u128 = 250;
/// If this many bytes of edits accumulate, flush immediately.
const FLUSH_SIZE_THRESHOLD: usize = 4096;

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

/// Shadow WAL journal — owns the `~/.xei/journal/` directory.
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
        Self {
            wal_dir: dir,
            tracked: HashMap::new(),
            last_flush: Instant::now(),
            pending_bytes: 0,
            last_version: 0,
            pending_recovery,
        }
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
    /// - `file_path`: current buffer path (empty = untitled).
    /// - `buffer_text`: full buffer content.
    /// - `buffer_version`: monotonically increasing edit counter.
    /// - `cursor_row`, `cursor_col`, `scroll`: viewport state to restore.
    /// - `dirty`: true if buffer has unsaved changes.
    pub fn on_tick(
        &mut self,
        file_path: &str,
        buffer_text: &str,
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
        let should_flush = elapsed >= FLUSH_INTERVAL_MS
            || self.pending_bytes >= FLUSH_SIZE_THRESHOLD;

        if !should_flush {
            return;
        }

        self.flush(file_path, buffer_text, cursor_row, cursor_col, scroll);
    }

    /// Called on explicit save (⌘S / :w) — the file is now durable, delete WAL.
    pub fn on_saved(&mut self, file_path: &str) {
        if let Some(name) = self.tracked.remove(file_path) {
            let wal_path = self.wal_dir.join(&name);
            let _ = fs::remove_file(wal_path);
        }
        self.pending_bytes = 0;
    }

    // ─── Internal ─────────────────────────────────────────────────────────────

    fn flush(
        &mut self,
        file_path: &str,
        buffer_text: &str,
        cursor_row: u32,
        cursor_col: u32,
        scroll: u32,
    ) {
        let name = Self::hash_name(file_path);
        let wal_path = self.wal_dir.join(&name);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let content = format!(
            "SUISEI-WAL v1\npath: {}\ncursor_row: {}\ncursor_col: {}\nscroll: {}\ntimestamp: {}\n---\n{}",
            file_path, cursor_row, cursor_col, scroll, timestamp, buffer_text
        );

        // Atomic write: temp → fsync → rename (never truncate the WAL mid-write).
        let tmp_path = wal_path.with_extension("tmp");
        if let Ok(mut f) = fs::File::create(&tmp_path) {
            if f.write_all(content.as_bytes()).is_ok() {
                let _ = f.sync_all();
                let _ = fs::rename(&tmp_path, &wal_path);
            }
        }

        self.tracked.insert(file_path.to_string(), name);
        self.last_flush = Instant::now();
        self.pending_bytes = 0;
    }

    fn wal_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".xei").join("journal")
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
        let Ok(dir) = fs::read_dir(wal_dir) else { return entries };
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
