//! Shared daemon state — the single source a [`Status`] snapshot reads from.
//!
//! Uptime and health are the daemon's own. The LSP/DAP/project fields are
//! **reported in** by editors over [`Opcode::ReportStatus`](crate::protocol::Opcode)
//! — the daemon does not own the language servers yet (D1 continues), but it
//! now owns the *bookkeeping*: one entry per connected editor, keyed by
//! client id. A snapshot AGGREGATES the live editors — sessions summed,
//! states taking the best, the project from whoever reported last — and a
//! disconnect drops its editor at once. The old single-slot state let two
//! open windows play last-writer-wins, and a killed window kept its stale
//! claim for the whole [`REPORT_TTL`]; per-client entries fix both.
//!
//! The TTL remains as a stall guard: a wedged editor (connected but silent)
//! ages out of the aggregate until it reports again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::Status;

/// Health values (mirrors [`Status::health`]).
pub mod health {
    pub const STARTING: u8 = 0;
    pub const HEALTHY: u8 = 1;
    pub const DEGRADED: u8 = 2;
}

/// How long an editor's report stays believable without a fresh one. The
/// editor re-reports on a heartbeat well inside this window (see the
/// engine's `daemon_report`), so only a wedged editor lets it lapse — a
/// DEAD one is dropped the moment its socket closes, no waiting.
pub const REPORT_TTL: Duration = Duration::from_secs(12);

/// Client id for state the daemon sets itself (the future in-daemon
/// managers). Behaves like an editor that never disconnects.
pub const LOCAL_CLIENT: u64 = u64::MAX;

/// One connected editor's latest report.
#[derive(Debug, Clone)]
struct EditorReport {
    lsp_sessions: u16,
    lsp_state: u8,
    dap_state: u8,
    project: String,
    /// When this editor last reported — the per-client stall clock.
    last_seen: Instant,
}

pub struct DaemonState {
    started: Instant,
    health: AtomicU8,
    /// One entry per connected editor (+ [`LOCAL_CLIENT`] for daemon-owned
    /// sources). Disconnect removes the entry — that is the primary
    /// lifecycle; [`REPORT_TTL`] only ages out the wedged.
    editors: Mutex<HashMap<u64, EditorReport>>,
}

impl DaemonState {
    pub fn new() -> Arc<Self> {
        Arc::new(DaemonState {
            started: Instant::now(),
            health: AtomicU8::new(health::HEALTHY),
            editors: Mutex::new(HashMap::new()),
        })
    }

    /// Build the current snapshot for a `StatusReport`: the aggregate of
    /// every editor still inside [`REPORT_TTL`]. Sessions sum, LSP/DAP
    /// states take the best (ready beats indexing beats starting), and the
    /// project comes from whoever reported most recently. No fresh editors
    /// → the zeros the menu bar renders as "none".
    pub fn status(&self) -> Status {
        let base = Status {
            health: self.health.load(Ordering::Relaxed),
            uptime_secs: self.started.elapsed().as_secs(),
            ..Status::default()
        };
        let editors = lock(&self.editors);
        let mut sessions: u16 = 0;
        let mut lsp_state: u8 = 0;
        let mut dap_state: u8 = 0;
        let mut project = String::new();
        let mut project_seen: Option<Instant> = None;
        for r in editors
            .values()
            .filter(|r| r.last_seen.elapsed() < REPORT_TTL)
        {
            sessions = sessions.saturating_add(r.lsp_sessions);
            lsp_state = lsp_state.max(r.lsp_state);
            dap_state = dap_state.max(r.dap_state);
            if !r.project.is_empty() && project_seen.map(|t| r.last_seen > t).unwrap_or(true) {
                project.clone_from(&r.project);
                project_seen = Some(r.last_seen);
            }
        }
        Status {
            lsp_sessions: sessions,
            lsp_state,
            dap_state,
            project,
            ..base
        }
    }

    /// True while at least one editor is still inside [`REPORT_TTL`].
    pub fn report_is_fresh(&self) -> bool {
        lock(&self.editors)
            .values()
            .any(|r| r.last_seen.elapsed() < REPORT_TTL)
    }

    /// How many editors currently have state on file (fresh or not).
    pub fn editor_count(&self) -> usize {
        lock(&self.editors).len()
    }

    /// Adopt an editor's [`Status`] push under its client id. `uptime_secs`
    /// and `health` are the daemon's own and are ignored — a client cannot
    /// declare the daemon sick.
    pub fn apply_report(&self, client: u64, reported: &Status) {
        let mut editors = lock(&self.editors);
        let slot = editors.entry(client).or_insert_with(|| EditorReport {
            lsp_sessions: 0,
            lsp_state: 0,
            dap_state: 0,
            project: String::new(),
            last_seen: Instant::now(),
        });
        slot.lsp_sessions = reported.lsp_sessions;
        slot.lsp_state = reported.lsp_state;
        slot.dap_state = reported.dap_state;
        slot.project.clone_from(&reported.project);
        slot.last_seen = Instant::now();
    }

    /// An editor disconnected: its state leaves the aggregate at once.
    /// Returns true when there was an entry to drop.
    pub fn remove_client(&self, client: u64) -> bool {
        lock(&self.editors).remove(&client).is_some()
    }

    // ── Daemon-local writers (the future in-daemon managers set their own
    // fields directly; they report under [`LOCAL_CLIENT`]). ────────────────
    pub fn set_lsp(&self, sessions: u16, state: u8) {
        let mut editors = lock(&self.editors);
        let slot = editors.entry(LOCAL_CLIENT).or_insert_with(|| EditorReport {
            lsp_sessions: 0,
            lsp_state: 0,
            dap_state: 0,
            project: String::new(),
            last_seen: Instant::now(),
        });
        slot.lsp_sessions = sessions;
        slot.lsp_state = state;
        slot.last_seen = Instant::now();
    }
    pub fn set_dap(&self, state: u8) {
        let mut editors = lock(&self.editors);
        if let Some(slot) = editors.get_mut(&LOCAL_CLIENT) {
            slot.dap_state = state;
            slot.last_seen = Instant::now();
        }
    }
    pub fn set_project(&self, path: &str) {
        let mut editors = lock(&self.editors);
        let slot = editors.entry(LOCAL_CLIENT).or_insert_with(|| EditorReport {
            lsp_sessions: 0,
            lsp_state: 0,
            dap_state: 0,
            project: String::new(),
            last_seen: Instant::now(),
        });
        slot.project.clear();
        slot.project.push_str(path);
        slot.last_seen = Instant::now();
    }
    /// Health is the daemon's own liveness, not a reported field — it never
    /// expires.
    pub fn set_health(&self, health: u8) {
        self.health.store(health, Ordering::Relaxed);
    }

    /// Test seam: age a client's report without waiting for the TTL.
    #[cfg(test)]
    fn backdate(&self, client: u64, by: Duration) {
        if let Some(r) = lock(&self.editors).get_mut(&client) {
            r.last_seen -= by;
        }
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

    fn report(sessions: u16, lsp_state: u8, dap_state: u8, project: &str) -> Status {
        Status {
            lsp_sessions: sessions,
            lsp_state,
            dap_state,
            project: project.to_string(),
            ..Status::default()
        }
    }

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
        s.apply_report(
            1,
            &Status {
                lsp_sessions: 1,
                lsp_state: 2, // indexing
                dap_state: 1,
                health: health::DEGRADED, // the client does not get to say this
                uptime_secs: 999_999,     // nor this
                project: "/Users/asill/suisei".to_string(),
            },
        );
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 1);
        assert_eq!(snap.lsp_state, 2);
        assert_eq!(snap.dap_state, 1);
        assert_eq!(snap.project, "/Users/asill/suisei");
        assert_eq!(snap.health, health::HEALTHY, "health is the daemon's own");
        assert!(snap.uptime_secs < 999_999, "uptime is the daemon's own");
    }

    /// Two open windows used to play last-writer-wins over one slot. Now the
    /// snapshot aggregates: sessions sum, the best state wins, and the
    /// project comes from whoever reported last.
    #[test]
    fn two_editors_aggregate_into_one_snapshot() {
        let s = DaemonState::new();
        s.apply_report(1, &report(1, 2, 0, "/tmp/alpha")); // indexing
        s.apply_report(2, &report(2, 3, 1, "/tmp/beta")); // ready, most recent
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 3, "sessions sum across editors");
        assert_eq!(snap.lsp_state, 3, "the best state wins");
        assert_eq!(snap.dap_state, 1);
        assert_eq!(
            snap.project, "/tmp/beta",
            "project follows the latest report"
        );
        assert_eq!(s.editor_count(), 2);
    }

    /// A closed window leaves at once — no twelve-second ghost in the menu
    /// bar while a live sibling keeps reporting.
    #[test]
    fn a_disconnect_drops_its_editor_immediately() {
        let s = DaemonState::new();
        s.apply_report(1, &report(1, 3, 0, "/tmp/alpha"));
        s.apply_report(2, &report(2, 3, 1, "/tmp/beta"));
        assert!(s.remove_client(2));
        assert!(!s.remove_client(2), "the second remove finds nothing");
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 1, "only the survivor counts");
        assert_eq!(snap.project, "/tmp/alpha");
        assert_eq!(snap.dap_state, 0, "the dead editor's DAP is gone");
        assert_eq!(s.editor_count(), 1);
    }

    /// The TTL is the stall guard: a wedged editor (connected but silent)
    /// ages out of the aggregate; a fresh sibling keeps shining.
    #[test]
    fn a_stale_editor_ages_out_of_the_aggregate() {
        let s = DaemonState::new();
        s.apply_report(1, &report(1, 3, 0, "/tmp/alpha"));
        s.apply_report(2, &report(2, 3, 1, "/tmp/beta"));
        s.backdate(1, REPORT_TTL + Duration::from_secs(1));
        assert!(s.report_is_fresh(), "editor 2 is still fresh");
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 2, "the wedged editor no longer counts");
        assert_eq!(snap.project, "/tmp/beta");
        // Everyone silent → the zeros the menu bar renders as "none", but
        // the daemon's own fields survive: it is still up.
        s.backdate(2, REPORT_TTL + Duration::from_secs(1));
        assert!(!s.report_is_fresh());
        let snap = s.status();
        assert_eq!(snap.lsp_sessions, 0);
        assert!(snap.project.is_empty());
        assert_eq!(snap.health, health::HEALTHY);
    }
}
