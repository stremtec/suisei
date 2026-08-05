//! Recovery FFI integration tests — end-to-end WAL → engine → FFI → accept/discard.
//!
//! These verify the C ABI recovery functions (`suisei_engine_recovery_count/path/
//! accept/discard`) through the actual `#[unsafe(no_mangle)]` entry points,
//! exercising the same path the Swift face will call on startup.
//!
//! ```text
//! cargo test -p suisei-engine --test recovery_ffi
//! ```

use std::ffi::c_char;
use suisei_engine::ffi::{
    SuiseiEngine, suisei_engine_recovery_accept, suisei_engine_recovery_count,
    suisei_engine_recovery_discard, suisei_engine_recovery_path,
};
use suisei_engine::runtime::Engine;

fn tmp_wal_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suisei_recovery_ffi_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build an Engine whose journal points at an isolated temp directory.
fn engine_with_journal(dir: &std::path::Path) -> Engine {
    let mut engine = Engine::new();
    engine.journal = suisei_engine::journal::Journal::with_dir(dir.to_path_buf());
    engine
}

/// Simulate a crash: write a WAL entry, then drop the engine.
fn simulate_crash(
    dir: &std::path::Path,
    file_path: &str,
    text: &str,
    row: usize,
    col: usize,
    scroll: usize,
) {
    let mut engine = engine_with_journal(dir);
    engine.app.filename = Some(std::path::PathBuf::from(file_path));
    engine.app.modified = true;
    engine.app.buffer = suisei_core::buffer::Buffer::from_string(text);
    engine.app.buffer.cursor.row = row;
    engine.app.buffer.cursor.col = col;
    engine.app.scroll = scroll;
    engine.app.buffer.set_version(1);
    // Two ticks with large version delta → size-threshold flush.
    engine.tick(50);
    engine.app.buffer.set_version(200);
    engine.tick(50);
}

#[test]
fn recovery_count_zero_on_clean_start() {
    let dir = tmp_wal_dir("clean");
    let engine = engine_with_journal(&dir);
    let ptr = &engine as *const _ as *const SuiseiEngine;
    let count = suisei_engine_recovery_count(ptr);
    assert_eq!(count, 0, "no recovery entries on a fresh journal dir");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_count_after_crash() {
    let dir = tmp_wal_dir("crash");
    simulate_crash(
        &dir,
        "/tmp/recovery_test.rs",
        "unsaved code\nline 2\n",
        0,
        0,
        0,
    );
    // Engine is dropped (crash). Fresh engine scans the same dir.
    let engine = engine_with_journal(&dir);
    let ptr = &engine as *const _ as *const SuiseiEngine;
    let count = suisei_engine_recovery_count(ptr);
    assert!(
        count >= 1,
        "at least one WAL entry after simulated crash, got {count}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_roundtrips_cursor_and_scroll() {
    // The P0.3 gate requires cursor AND scroll to survive a crash. Assert the
    // WAL write → scan → parse round-trip preserves all three, deterministically
    // — independent of the post-accept recompose that keeps the caret on-screen
    // (which can legitimately adjust `scroll` for a tiny doc).
    let dir = tmp_wal_dir("viewport");
    simulate_crash(&dir, "/tmp/viewport_test.rs", "a\nb\nc\nd\ne\nf\n", 4, 1, 2);
    let engine = engine_with_journal(&dir);
    let entry = engine
        .journal
        .recovery_entry(0)
        .expect("one recovery entry after crash");
    assert_eq!(
        entry.cursor_row, 4,
        "cursor row round-trips through the WAL"
    );
    assert_eq!(
        entry.cursor_col, 1,
        "cursor col round-trips through the WAL"
    );
    assert_eq!(entry.scroll, 2, "scroll round-trips through the WAL");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_path_returns_filepath() {
    let dir = tmp_wal_dir("path");
    simulate_crash(&dir, "/tmp/path_test.rs", "content", 0, 0, 0);

    let engine = engine_with_journal(&dir);
    let ptr = &engine as *const _ as *const SuiseiEngine;
    let count = suisei_engine_recovery_count(ptr);
    assert!(count >= 1);

    let mut buf = [0u8; 512];
    let ok = suisei_engine_recovery_path(ptr, 0, buf.as_mut_ptr() as *mut c_char, buf.len() as u32);
    assert_eq!(ok, 1, "recovery_path should return 1 for valid idx");

    let path = std::str::from_utf8(&buf)
        .unwrap()
        .trim_end_matches('\0')
        .to_string();
    assert!(
        path.contains("path_test.rs"),
        "recovered path should contain the original filename, got: {path}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_accept_restores_buffer_and_clears_entry() {
    let dir = tmp_wal_dir("accept");
    simulate_crash(
        &dir,
        "/tmp/accept_test.rs",
        "recovered text\nsecond line\n",
        1,
        6,
        3,
    );

    let mut engine = engine_with_journal(&dir);
    let cptr = &engine as *const _ as *const SuiseiEngine;
    let count_before = suisei_engine_recovery_count(cptr);
    assert!(count_before >= 1, "recovery entry should exist");

    let mptr = &mut engine as *mut _ as *mut SuiseiEngine;
    let ok = suisei_engine_recovery_accept(mptr, 0);
    assert_eq!(ok, 1, "recovery_accept should return 1 on success");

    let text = engine.app.buffer.text();
    assert!(
        text.contains("recovered text"),
        "buffer should contain recovered text after accept, got: {text}"
    );
    assert!(
        engine.app.modified,
        "buffer should be marked modified after recovery"
    );
    assert_eq!(
        engine.app.buffer.cursor.row, 1,
        "cursor row should be restored"
    );
    assert_eq!(
        engine.app.buffer.cursor.col, 6,
        "cursor col should be restored"
    );

    let cptr = &engine as *const _ as *const SuiseiEngine;
    let count_after = suisei_engine_recovery_count(cptr);
    assert_eq!(
        count_after,
        count_before - 1,
        "recovery count should decrease by 1 after accept"
    );

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "WAL file should be deleted after accept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_accept_clamps_cursor_col_past_line_end() {
    // A col recovered from a crash may land past the end of a now-shorter line.
    // Accept must clamp it, never leave the caret out of bounds (a later edit
    // or paint would then panic).
    let dir = tmp_wal_dir("clamp");
    simulate_crash(&dir, "/tmp/clamp_test.rs", "hi\n", 0, 99, 0);
    let mut engine = engine_with_journal(&dir);
    let mptr = &mut engine as *mut _ as *mut SuiseiEngine;
    assert_eq!(suisei_engine_recovery_accept(mptr, 0), 1);
    let len = engine
        .app
        .buffer
        .line(engine.app.buffer.cursor.row)
        .chars()
        .count();
    assert!(
        engine.app.buffer.cursor.col <= len,
        "col {} must clamp to line length {len}",
        engine.app.buffer.cursor.col
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_discard_deletes_wal_and_clears_entry() {
    let dir = tmp_wal_dir("discard_ffi");
    simulate_crash(&dir, "/tmp/discard_ffi.rs", "to be discarded", 0, 0, 0);

    let mut engine = engine_with_journal(&dir);
    let ptr = &engine as *const _ as *const SuiseiEngine;
    let count = suisei_engine_recovery_count(ptr);
    assert!(count >= 1);

    let mptr = &mut engine as *mut _ as *mut SuiseiEngine;
    suisei_engine_recovery_discard(mptr, 0);

    let cptr = &engine as *const _ as *const SuiseiEngine;
    let count_after = suisei_engine_recovery_count(cptr);
    assert_eq!(
        count_after,
        count - 1,
        "count should decrease after discard"
    );

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "no WAL files after discard");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_path_out_of_range_returns_zero() {
    let dir = tmp_wal_dir("oor");
    let engine = engine_with_journal(&dir);
    let ptr = &engine as *const _ as *const SuiseiEngine;
    let mut buf = [0u8; 64];
    let ok =
        suisei_engine_recovery_path(ptr, 999, buf.as_mut_ptr() as *mut c_char, buf.len() as u32);
    assert_eq!(ok, 0, "out-of-range idx should return 0");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_accept_out_of_range_returns_zero() {
    let dir = tmp_wal_dir("accept_oor");
    let mut engine = engine_with_journal(&dir);
    let ptr = &mut engine as *mut _ as *mut SuiseiEngine;
    let ok = suisei_engine_recovery_accept(ptr, 999);
    assert_eq!(ok, 0, "out-of-range accept should return 0");
    let _ = std::fs::remove_dir_all(&dir);
}
