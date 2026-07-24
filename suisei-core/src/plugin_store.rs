//! Plugin store surface state (v2). Plain, frontend-agnostic: the App feeds it
//! (installed list + async Open VSX search/install results via a channel), the
//! renderer draws it, event handling drives navigation. Network work happens on
//! background threads in the App (feature-gated); this module only holds state
//! and drains the results channel — no I/O of its own.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Idle time after the last keystroke before a live search fires.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(280);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StoreTab {
    #[default]
    Installed,
    Browse,
}

/// One row in the store (installed or a search result).
#[derive(Clone, Debug)]
pub struct StoreItem {
    pub id: String, // publisher.name
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed: bool,
    /// Fidelity badge once profiled: 🟢 full / 🟡 degraded / 🔴 GUI-only.
    pub fidelity: Option<char>,
    pub downloads: u64,
}

/// One render row of the Extensions sidebar panel: either an extension header
/// or a runnable command under it.
#[derive(Clone, Debug)]
pub struct ExtRow {
    pub is_header: bool,
    pub primary: String,
    pub secondary: String,
    /// `Some(command_id)` when this row runs a command (Enter).
    pub command: Option<String>,
}

/// Async result delivered from a background thread.
pub enum StoreMsg {
    Results(Vec<StoreItem>),
    Installed { id: String, version: String },
    Error(String),
}

#[derive(Default)]
pub struct PluginStore {
    pub open: bool,
    pub tab: StoreTab,
    pub installed: Vec<StoreItem>,
    pub results: Vec<StoreItem>,
    pub selected: usize,
    /// Search query; `input` = typing into it.
    pub query: String,
    pub input: bool,
    pub loading: bool,
    pub message: String,
    /// Set when an install just completed, so the frontend can (re)load the host.
    pub just_installed: bool,
    rx: Option<Receiver<StoreMsg>>,
    /// Live-search debounce: set on each keystroke, cleared when a search fires.
    dirty_since: Option<Instant>,
    /// The query most recently dispatched (dedupes live-search fires).
    last_searched: String,
}

impl PluginStore {
    pub fn open(&mut self, installed: Vec<StoreItem>) {
        self.open = true;
        self.tab = StoreTab::Installed;
        self.installed = installed;
        self.selected = 0;
        self.input = false;
        self.message = "j/k move · Tab switch · / search · Enter install · x uninstall · Esc close".into();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input = false;
        self.loading = false;
        self.rx = None;
    }

    /// Rows currently shown (depends on the active tab).
    pub fn rows(&self) -> &[StoreItem] {
        match self.tab {
            StoreTab::Installed => &self.installed,
            StoreTab::Browse => &self.results,
        }
    }

    pub fn selected_item(&self) -> Option<&StoreItem> {
        self.rows().get(self.selected)
    }

    pub fn switch_tab(&mut self) {
        self.tab = match self.tab {
            StoreTab::Installed => StoreTab::Browse,
            StoreTab::Browse => StoreTab::Installed,
        };
        self.selected = 0;
    }

    pub fn move_sel(&mut self, delta: isize) {
        let len = self.rows().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected as isize + delta;
        self.selected = cur.clamp(0, len as isize - 1) as usize;
    }

    // ── live search input ──
    pub fn begin_input(&mut self) {
        self.tab = StoreTab::Browse;
        self.input = true;
        self.message = "live search · results update as you type · Enter to browse · Esc".into();
    }
    pub fn input_char(&mut self, c: char) {
        self.query.push(c);
        self.dirty_since = Some(Instant::now());
    }
    pub fn input_backspace(&mut self) {
        self.query.pop();
        self.dirty_since = Some(Instant::now());
    }
    /// Leave the query field to navigate results (query + results stay).
    pub fn cancel_input(&mut self) {
        self.input = false;
    }

    /// True when the debounced query has settled and differs from the last one
    /// dispatched — the frontend fires a search and calls [`mark_searched`].
    pub fn search_due(&self) -> bool {
        let q = self.query.trim();
        !q.is_empty()
            && q != self.last_searched
            && self.dirty_since.map(|t| t.elapsed() >= SEARCH_DEBOUNCE).unwrap_or(false)
    }

    pub fn mark_searched(&mut self) {
        self.last_searched = self.query.trim().to_string();
        self.dirty_since = None;
    }

    /// Attach a background job's result channel and enter the loading state.
    pub fn begin_job(&mut self, rx: Receiver<StoreMsg>, note: &str) {
        self.rx = Some(rx);
        self.loading = true;
        self.message = note.to_string();
    }

    /// Drain any async result. Returns true if state changed (redraw hint).
    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.rx.as_ref() else {
            return false;
        };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(StoreMsg::Results(items)) => {
                    self.results = items;
                    self.selected = 0;
                    self.loading = false;
                    self.message = format!("{} result(s) · Enter to install", self.results.len());
                    changed = true;
                }
                Ok(StoreMsg::Installed { id, version }) => {
                    self.loading = false;
                    self.just_installed = true;
                    self.message = format!("installed {id} v{version} — loading…");
                    changed = true;
                }
                Ok(StoreMsg::Error(e)) => {
                    self.loading = false;
                    self.message = format!("error: {e}");
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.loading = false;
                    break;
                }
            }
        }
        if self.rx.is_some() && !self.loading {
            self.rx = None;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_search_debounces_and_dedupes() {
        let mut s = PluginStore::default();
        s.begin_input();
        s.input_char('r');
        s.input_char('s');
        assert!(!s.search_due(), "should wait for the debounce window");
        std::thread::sleep(SEARCH_DEBOUNCE + Duration::from_millis(30));
        assert!(s.search_due(), "should be due after the debounce");
        s.mark_searched();
        assert!(!s.search_due(), "same query must not re-fire");
        assert_eq!(s.last_searched, "rs");
        s.input_char('t'); // query changed → eligible again after debounce
        std::thread::sleep(SEARCH_DEBOUNCE + Duration::from_millis(30));
        assert!(s.search_due());
    }
}
