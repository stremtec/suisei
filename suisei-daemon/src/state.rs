//! Shared daemon state — the single source a [`Status`] snapshot reads from.
//!
//! Uptime and health are the daemon's own. The LSP/DAP/project fields are
//! **reported in** by the editor over [`Opcode::ReportStatus`](crate::protocol::Opcode),
//! because the daemon does not own the language servers yet (D1). A reported
//! field is therefore only as good as its last delivery, so it **expires**: see
//! [`REPORT_TTL`]. Without that, killing the GUI mid-session would leave the
//! menu bar cheerfully claiming "ready (1)" forever.

use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::Status;

/// Health values (mirrors [`Status::health`]).
pub mod health {
    pub const STARTING: u8 = 0;
    pub const HEALTHY: u8 = 1;
    pub const DEGRADED: u8 = 2;
}

/// How long the LSP/DAP/project fields stay believable after their last write.
/// The editor re-reports on a heartbeat well inside this window (see the
/// engine's `daemon_report`), so only a dead or wedged editor lets it lapse.
pub const REPORT_TTL: Duration = Duration::from_secs(12);

pub struct DaemonState {
    started: Instant,
    lsp_sessions: AtomicU16,
    lsp_state: AtomicU8,
    dap_state: AtomicU8,
    health: AtomicU8,
    project: Mutex<String>,
    /// When the editor last reported. `None` = never; nothing to believe.
    last_report: Mutex<Option<Instant>>,
}

impl DaemonState {
    pub fn new() -> Arc<Self> {
        Arc::new(DaemonState {
            started: Instant::now(),
            lsp_sessions: AtomicU16::new(0),
            lsp_state: AtomicU8::new(0),
            dap_state: AtomicU8::new(0),
            health: AtomicU8::new(health::HEALTHY),
            project: Mutex::new(String::new()),
            last_report: Mutex::new(None),
        })
    }

    /// Build the current snapshot for a `StatusReport`. Reported fields decay to
    /// "nothing" once [`REPORT_TTL`] passes without a fresh report.
    pub fn status(&self) -> Status {
        let base = Status {
            health: self.health.load(Ordering::Relaxed),
            uptime_secs: self.started.elapsed().as_secs(),
            ..Status::default()
        };
        if !self.report_is_fresh() {
            return base;
        }
        Status {
            lsp_sessions: self.lsp_sessions.load(Ordering::Relaxed),
            lsp_state: self.lsp_state.load(Ordering::Relaxed),
            dap_state: self.dap_state.load(Ordering::Relaxed),
            project: lock(&self.project).clone(),
            ..base
        }
    }

    /// True while the last editor report is still inside [`REPORT_TTL`].
    pub fn report_is_fresh(&self) -> bool {
        matches!(*lock(&self.last_report), Some(at) if at.elapsed() < REPORT_TTL)
    }

    /// Adopt an editor's [`Status`] push. `uptime_secs` and `health` are the
    /// daemon's own and are ignored — a client cannot declare the daemon sick.
    pub fn apply_report(&self, reported: &Status) {
        self.set_lsp(reported.lsp_sessions, reported.lsp_state);
        self.set_dap(reported.dap_state);
        self.set_project(&reported.project);
    }

    // ── Field writers. Each one refreshes the TTL: a value is believable for
    // `REPORT_TTL` after whoever owns it last wrote it, whether that is the
    // editor over the socket or an in-daemon manager later. ───────────────────
    pub fn set_lsp(&self, sessions: u16, state: u8) {
        self.lsp_sessions.store(sessions, Ordering::Relaxed);
        self.lsp_state.store(state, Ordering::Relaxed);
        self.touch();
    }
    pub fn set_dap(&self, state: u8) {
        self.dap_state.store(state, Ordering::Relaxed);
        self.touch();
    }
    pub fn set_project(&self, path: &str) {
        let mut p = lock(&self.project);
        p.clear();
        p.push_str(path);
        drop(p);
        self.touch();
    }
    /// Health is the daemon's own liveness, not a reported field — it never
    /// expires and so does not refresh the TTL.
    pub fn set_health(&self, health: u8) {
        self.health.store(health, Ordering::Relaxed);
    }

    fn touch(&self) {
        *lock(&self.last_report) = Some(Instant::now());
    }
}

/// A poisoned mutex here means some other thread panicked mid-update; the state
/// is plain data, so recovering the inner value beats taking the daemon down.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_healthy_with_no_sessions() {
        let s = DaemonState::new();
        let snap = s.status();
        assert_eq!(snap.health, health::HEALTHY);
        assert_eq!(snap.lsp_sessions, 0);
        assert_eq!(snap.lsp_state, 0);
        assert_eq!(snap.dap_state, 0);
        assert!(snap.project.is_empty());
    }

    #[test]
    fn setters_flow_into_the_snapshot() {
        let s = DaemonState::new();
        s.set_lsp(1, 3); // one session, ready
        s.set_dap(1); // running
        s.set_project("/Users/asill/suisei");
        s.set_health(health::DEGRADED);
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 1);
        assert_eq!(snap.lsp_state, 3);
        assert_eq!(snap.dap_state, 1);
        assert_eq!(snap.health, health::DEGRADED);
        assert_eq!(snap.project, "/Users/asill/suisei");
    }

    #[test]
    fn an_editor_report_becomes_the_snapshot() {
        let s = DaemonState::new();
        s.apply_report(&Status {
            lsp_sessions: 1,
            lsp_state: 2, // indexing
            dap_state: 1,
            health: health::DEGRADED, // the client does not get to say this
            uptime_secs: 999_999,     // nor this
            project: "/Users/asill/suisei".to_string(),
        });
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 1);
        assert_eq!(snap.lsp_state, 2);
        assert_eq!(snap.dap_state, 1);
        assert_eq!(snap.project, "/Users/asill/suisei");
        assert_eq!(snap.health, health::HEALTHY, "health is the daemon's own");
        assert!(snap.uptime_secs < 999_999, "uptime is the daemon's own");
    }

    /// Without expiry a dead editor would leave the menu bar claiming a live
    /// language server forever.
    #[test]
    fn a_stale_report_decays_to_nothing() {
        let s = DaemonState::new();
        s.apply_report(&Status {
            lsp_sessions: 1,
            lsp_state: 3,
            dap_state: 2,
            project: "/Users/asill/suisei".to_string(),
            ..Status::default()
        });
        assert!(s.report_is_fresh());
        // Backdate the last report past the TTL.
        *s.last_report.lock().unwrap() = Some(Instant::now() - REPORT_TTL - Duration::from_secs(1));

        assert!(!s.report_is_fresh());
        let snap = s.status();
        assert_eq!(snap.lsp_state, 0);
        assert_eq!(snap.lsp_sessions, 0);
        assert_eq!(snap.dap_state, 0);
        assert!(snap.project.is_empty());
        // The daemon's own fields survive: it is still up.
        assert_eq!(snap.health, health::HEALTHY);
    }
}
