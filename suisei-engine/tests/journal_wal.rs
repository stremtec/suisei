//! Shadow WAL journal — write, recover, accept/discard cycle tests.
//!
//! ```text
//! cargo test -p suisei-engine --test journal_wal
//! ```

use suisei_engine::journal::Journal;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suisei_wal_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn wal_write_and_scan() {
    let dir = tmp_dir("write");
    let mut journal = Journal::with_dir(dir.clone());

    let text = "fn main() {\n    println!(\"hello\");\n}\n".repeat(200);
    let path = "/tmp/test_file.rs";

    // First tick: establishes baseline version.
    journal.on_tick(path, || text.clone(), 1, 5, 3, 10, true);
    // Second tick: delta = 99, pending_bytes = 99*64 = 6336 > 4096 → flush.
    journal.on_tick(path, || text.clone(), 100, 5, 3, 10, true);

    // Find the WAL file.
    let wal_files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .collect();
    assert_eq!(wal_files.len(), 1, "exactly one WAL file after flush");

    // Verify WAL format.
    let content = std::fs::read_to_string(wal_files[0].path()).unwrap();
    assert!(content.starts_with("SUISEI-WAL v1\n"), "WAL magic header");
    assert!(content.contains(&format!("path: {path}\n")), "WAL path field");
    assert!(content.contains("cursor_row: 5\n"), "WAL cursor_row");
    assert!(content.contains("cursor_col: 3\n"), "WAL cursor_col");
    assert!(content.contains("scroll: 10\n"), "WAL scroll");
    assert!(content.contains("---\n"), "WAL separator");
    assert!(content.contains("fn main()"), "WAL contains buffer text");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wal_saved_removes_entry() {
    let dir = tmp_dir("saved");
    let mut journal = Journal::with_dir(dir.clone());

    let path = "/tmp/saved_file.rs";
    let text = "saved content";

    // Force a flush (size threshold).
    journal.on_tick(path, || text.to_string(), 1, 0, 0, 0, true);
    journal.on_tick(path, || text.to_string(), 100, 0, 0, 0, true);

    // Save: should remove the journal entry.
    journal.on_saved(path);

    // WAL file should be gone.
    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "no WAL files after save");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_scan_and_accept() {
    let dir = tmp_dir("recover");
    let path = "/tmp/recover_me.rs";
    let text = "unsaved work that was lost in a crash";

    // Write a WAL entry.
    {
        let mut journal = Journal::with_dir(dir.clone());
        journal.on_tick(path, || text.to_string(), 1, 10, 5, 20, true);
        journal.on_tick(path, || text.to_string(), 200, 10, 5, 20, true);
    }

    // Simulate restart: fresh journal scans the same directory.
    let journal2 = Journal::with_dir(dir.clone());
    let count = journal2.recovery_count();
    assert!(count >= 1, "at least one recovery entry after crash");

    // Find our file.
    let mut found = false;
    for i in 0..count {
        if let Some(entry) = journal2.recovery_entry(i) {
            if entry.file_path == path {
                assert_eq!(entry.cursor_row, 10);
                assert_eq!(entry.cursor_col, 5);
                assert_eq!(entry.scroll, 20);
                assert!(entry.text.contains("unsaved work"));
                found = true;

                // Accept: returns the entry, deletes the WAL file.
                let mut journal3 = journal2;
                let accepted = journal3.accept_recovery(i);
                assert!(accepted.is_some(), "accept returns the entry");
                assert_eq!(accepted.unwrap().file_path, path);
                assert_eq!(journal3.recovery_count(), count - 1, "count decreases");
                break;
            }
        }
    }
    assert!(found, "recovery entry for {path} found");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_discard_removes_wal() {
    let dir = tmp_dir("discard");
    let path = "/tmp/discard_me.rs";

    // Write a WAL entry.
    {
        let mut journal = Journal::with_dir(dir.clone());
        journal.on_tick(path, || "discarded work".to_string(), 1, 0, 0, 0, true);
        journal.on_tick(path, || "discarded work".to_string(), 100, 0, 0, 0, true);
    }

    let mut journal2 = Journal::with_dir(dir.clone());
    let count = journal2.recovery_count();
    assert!(count >= 1);

    // Discard the first entry.
    journal2.discard_recovery(0);
    assert_eq!(journal2.recovery_count(), count - 1);

    // WAL file should be gone from disk.
    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "no WAL files after discard");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_buffer_not_journaled() {
    let dir = tmp_dir("clean");
    let mut journal = Journal::with_dir(dir.clone());

    // dirty=false → no WAL write.
    journal.on_tick("/tmp/clean.rs", || "clean text".to_string(), 1, 0, 0, 0, false);

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "clean buffer not journaled");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn untitled_not_journaled() {
    let dir = tmp_dir("untitled");
    let mut journal = Journal::with_dir(dir.clone());

    // Empty path → no WAL write.
    journal.on_tick("", || "unsaved text".to_string(), 1, 0, 0, 0, true);

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "untitled buffer not journaled");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The journal runs on the 20 Hz tick but flushes at most every 250 ms, and
/// only while dirty. Building the document for the ticks in between taxed every
/// session: 0.24 ms per tick on a 60,000-line file, thrown away immediately.
#[test]
fn a_tick_that_will_not_flush_never_builds_the_document() {
    let dir = tmp_dir("lazy");
    let mut journal = Journal::with_dir(dir);
    let mut builds = 0usize;

    journal.on_tick(
        "/tmp/lazy.rs",
        || {
            builds += 1;
            "text".to_string()
        },
        1,
        0,
        0,
        0,
        false, // clean
    );
    assert_eq!(builds, 0, "a clean buffer must not pay for a document build");

    // Dirty, but the first tick only establishes the version baseline and the
    // 250 ms debounce has not elapsed — still nothing to write.
    journal.on_tick(
        "/tmp/lazy.rs",
        || {
            builds += 1;
            "text".to_string()
        },
        1,
        0,
        0,
        0,
        true,
    );
    assert_eq!(builds, 0, "a tick inside the debounce must not build it either");
}
