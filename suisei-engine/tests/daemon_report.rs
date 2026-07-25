//! End to end: engine tick → Unix socket → daemon state.
//!
//! Every layer of this pipe had unit tests and the whole thing still reported
//! nothing, because the two halves were never joined — the daemon's setters had
//! no production caller. This test is the join: a real daemon on a real socket,
//! a real `Engine`, and an assertion that the snapshot the menu-bar agent polls
//! actually carries the editor's state.
//!
//! Its own process (integration tests get one binary each), so overriding
//! `XDG_RUNTIME_DIR` here cannot reach the developer's live daemon.

use std::sync::Arc;
use std::time::{Duration, Instant};

use suisei_daemon::{protocol, server, state::DaemonState};
use suisei_engine::Engine;

#[test]
fn the_engine_reports_its_live_state_into_a_running_daemon() {
    // Short path: macOS caps `sun_path` at ~104 bytes.
    let runtime_dir = std::path::PathBuf::from(format!("/tmp/suisei-rep-{}", std::process::id()));
    std::fs::create_dir_all(runtime_dir.join("suisei")).unwrap();
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir) };

    let sock = protocol::socket_path();
    let listener = server::bind(&sock).unwrap();
    let state = DaemonState::new();
    {
        let state = Arc::clone(&state);
        std::thread::spawn(move || server::serve(listener, state));
    }

    // A daemon nobody reports into says "none" for everything — the bug.
    let before = state.status();
    assert_eq!(before.lsp_state, 0);
    assert_eq!(before.dap_state, 0);
    assert!(before.project.is_empty());

    let project = runtime_dir.join("proj");
    std::fs::create_dir_all(&project).unwrap();

    let mut eng = Engine::new();
    eng.app.lsp.server_running = true; // stands in for a handshaked server
    eng.app.lsp.set_progress_open_for_test("rustAnalyzer/Indexing", true);
    eng.app.dap.state = suisei_core::dap::DapState::Stopped;
    eng.app.explorer.cwd = project.clone();
    eng.app.explorer.entries.push(suisei_core::explorer::ExplorerEntry {
        name: "a.rs".into(),
        path: project.join("a.rs"),
        is_dir: false,
    });
    eng.start_daemon_reporting();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = state.status();
    while Instant::now() < deadline && seen.lsp_state == 0 {
        eng.tick(50);
        std::thread::sleep(Duration::from_millis(5));
        seen = state.status();
    }

    assert_eq!(seen.lsp_state, 2, "an open progress token means indexing");
    assert_eq!(seen.lsp_sessions, 1);
    assert_eq!(seen.dap_state, 2, "stopped at a breakpoint reads as paused");
    assert_eq!(seen.project, project.display().to_string());

    // The daemon's own fields are still its own.
    assert_eq!(seen.health, suisei_daemon::state::health::HEALTHY);

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_dir_all(&runtime_dir);
}
