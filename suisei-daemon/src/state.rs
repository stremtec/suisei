//! Shared daemon state — the single source a [`Status`] snapshot reads from.
//!
//! Today it reports daemon aliveness (uptime) and health, with placeholders for
//! LSP/DAP/project. As the LSP and DAP managers land they call the setters here,
//! and the same status pipe the menu-bar agent already polls starts carrying
//! real data — no protocol change.

use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::protocol::Status;

/// Health values (mirrors [`Status::health`]).
pub mod health {
    pub const STARTING: u8 = 0;
    pub const HEALTHY: u8 = 1;
    pub const DEGRADED: u8 = 2;
}

pub struct DaemonState {
    started: Instant,
    lsp_sessions: AtomicU16,
    lsp_state: AtomicU8,
    dap_state: AtomicU8,
    health: AtomicU8,
    project: Mutex<String>,
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
        })
    }

    /// Build the current snapshot for a `StatusReport`.
    pub fn status(&self) -> Status {
        Status {
            lsp_sessions: self.lsp_sessions.load(Ordering::Relaxed),
            lsp_state: self.lsp_state.load(Ordering::Relaxed),
            dap_state: self.dap_state.load(Ordering::Relaxed),
            health: self.health.load(Ordering::Relaxed),
            uptime_secs: self.started.elapsed().as_secs(),
            project: self.project.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    // ── Setters used by the managers as they land ─────────────────────────────
    pub fn set_lsp(&self, sessions: u16, state: u8) {
        self.lsp_sessions.store(sessions, Ordering::Relaxed);
        self.lsp_state.store(state, Ordering::Relaxed);
    }
    pub fn set_dap(&self, state: u8) {
        self.dap_state.store(state, Ordering::Relaxed);
    }
    pub fn set_project(&self, path: &str) {
        if let Ok(mut p) = self.project.lock() {
            *p = path.to_string();
        }
    }
    pub fn set_health(&self, health: u8) {
        self.health.store(health, Ordering::Relaxed);
    }
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
}
