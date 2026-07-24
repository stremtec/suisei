//! The daemon owns the menu-bar agent's *presence*. The daemon is headless
//! Rust and cannot draw UI (arch plan §1.5), so it launches the lightweight
//! `SuiseiDaemonAgent.app`, which subscribes to this daemon's socket and shows
//! the status. macOS LaunchServices dedupes, so re-opening an already-running
//! agent is a no-op — that lets a supervisor thread keep it alive cheaply.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// How often the supervisor re-opens the agent (a no-op while it is alive,
/// a respawn after it has died or been quit).
const SUPERVISE_INTERVAL: Duration = Duration::from_secs(20);

/// Resolve the agent `.app`: `$SUISEI_AGENT_APP` first (dev override), else a
/// sibling of the daemon binary — production bundles both in the app's
/// `Contents/Helpers/`.
pub fn agent_app_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SUISEI_AGENT_APP") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let sibling = exe.parent()?.join("SuiseiDaemonAgent.app");
    sibling.exists().then_some(sibling)
}

/// Open the menu-bar agent in the background (`-g`, no focus steal). Idempotent
/// via LaunchServices. Returns false when no agent app could be found.
pub fn launch_agent() -> bool {
    let Some(app) = agent_app_path() else {
        eprintln!(
            "suisei-daemon: no agent app (set SUISEI_AGENT_APP or bundle it beside the daemon); \
             menu-bar status disabled"
        );
        return false;
    };
    match Command::new("open").arg("-g").arg("-a").arg(&app).status() {
        Ok(s) if s.success() => true,
        Ok(s) => {
            eprintln!("suisei-daemon: agent launch exited with {s}");
            false
        }
        Err(e) => {
            eprintln!("suisei-daemon: failed to launch agent: {e}");
            false
        }
    }
}

/// Launch the agent now and keep it alive: a background thread re-opens it on a
/// slow interval, so a crashed or user-quit agent comes back without needing
/// the daemon to restart.
pub fn supervise_agent() {
    if launch_agent() {
        eprintln!("suisei-daemon: menu-bar agent up");
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(SUPERVISE_INTERVAL);
        let _ = launch_agent();
    });
}
