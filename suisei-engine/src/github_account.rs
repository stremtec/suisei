//! GitHub identity for the Settings account page.
//!
//! Lives on the engine, not in chrome: the face only pulls this when Settings
//! is open, and a profile refresh must never republish the editor tree.
//! Login reuses the existing `gh auth login --web` session from `suisei_core::gh`.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use suisei_core::gh::{
    self, AuthLoginSession, GhAuthInfo, GhAuthState, GhContributions, GhProfile,
};

/// What a background probe produced.
enum AccountFetch {
    Ready {
        info: GhAuthInfo,
        profile: Option<GhProfile>,
        contributions: GhContributions,
        message: String,
    },
    Calendar {
        contributions: GhContributions,
        year: u32,
    },
}

pub struct GitHubAccount {
    pub generation: u64,
    pub info: GhAuthInfo,
    pub profile: GhProfile,
    pub contributions: GhContributions,
    /// 0 = rolling last 365 days.
    pub contrib_year: u32,
    pub contrib_year_min: u32,
    pub loading: bool,
    pub message: String,
    login: Option<AuthLoginSession>,
    /// The account fetch (auth status, profile, first calendar).
    ///
    /// Separate from the calendar's own slot, and that is the point. Both
    /// `spawn_fetch_with` and `set_contrib_year` used to assign the SAME field,
    /// so starting either while the other was in flight dropped the first
    /// receiver on the floor. Its thread's `send` then failed silently, its
    /// result never arrived, and for the account fetch that means `loading`
    /// stays true for the life of the process — a spinner under the portrait
    /// with nothing that can clear it. Changing the contribution year shortly
    /// after signing in is exactly that sequence, and signing in is when the
    /// year picker first has years to offer.
    fetch_rx: Option<Receiver<AccountFetch>>,
    /// A year the user picked, which can be asked for while the account fetch
    /// is still running and must not displace it.
    calendar_rx: Option<Receiver<AccountFetch>>,
    ever_loaded: bool,
}

impl Default for GitHubAccount {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubAccount {
    pub fn new() -> Self {
        Self {
            generation: 0,
            info: GhAuthInfo::default(),
            profile: GhProfile::default(),
            contributions: GhContributions::default(),
            contrib_year: 0,
            contrib_year_min: 0,
            loading: false,
            message: String::new(),
            login: None,
            fetch_rx: None,
            calendar_rx: None,
            ever_loaded: false,
        }
    }

    pub fn signing_in(&self) -> bool {
        self.login.is_some()
    }

    pub fn device_code(&self) -> &str {
        self.login
            .as_ref()
            .and_then(|s| s.code.as_deref())
            .unwrap_or("")
    }

    /// First Settings pull should not wait for a button. Later pulls are
    /// cheap: generation only moves when something actually changed.
    pub fn ensure_loaded(&mut self) {
        if !self.ever_loaded && !self.loading && self.login.is_none() {
            self.refresh();
        }
    }

    pub fn refresh(&mut self) {
        if self.login.is_some() {
            self.message = "Finish or cancel browser sign-in first".into();
            self.bump();
            return;
        }
        self.spawn_fetch("Refreshing GitHub account…");
    }

    pub fn sign_in(&mut self) {
        if self.login.is_some() {
            return;
        }
        if !gh::gh_installed() {
            self.info = GhAuthInfo {
                state: GhAuthState::NotInstalled,
                detail: "gh CLI not installed".into(),
                ..Default::default()
            };
            self.message = self.info.detail.clone();
            self.bump();
            return;
        }
        match gh::auth_login_web_start() {
            Ok(session) => {
                let _ = gh::open_in_browser("https://github.com/login/device");
                self.message = "Browser opened · waiting for GitHub…".into();
                self.login = Some(session);
                self.bump();
            }
            Err(e) => {
                self.message = e;
                self.bump();
            }
        }
    }

    pub fn cancel_sign_in(&mut self) {
        if let Some(session) = self.login.take() {
            session.cancel();
            self.message = "Sign-in cancelled".into();
            self.bump();
        }
    }

    pub fn sign_out(&mut self) {
        if self.login.is_some() {
            self.cancel_sign_in();
        }
        self.spawn_fetch_with("Signing out…", Some(|| {
            let logout = gh::auth_logout();
            let info = gh::auth_status();
            let profile = if info.state == GhAuthState::LoggedIn {
                gh::fetch_profile()
            } else {
                None
            };
            let message = match logout {
                Ok(msg) => msg,
                Err(e) => e,
            };
            AccountFetch::Ready {
                info,
                profile,
                contributions: GhContributions::default(),
                message,
            }
        }));
    }

    pub fn set_contrib_year(&mut self, year: u32) {
        if year == self.contrib_year && !self.contributions.levels.is_empty() {
            return;
        }
        self.contrib_year = year;
        self.bump();
        self.spawn_calendar(move || {
            let year_opt = if year == 0 { None } else { Some(year) };
            AccountFetch::Calendar {
                contributions: gh::fetch_contributions(year_opt).unwrap_or_default(),
                year,
            }
        });
    }

    /// Fetch one year's calendar in the background.
    ///
    /// Its own slot, not `fetch_rx`. Assigning that one here dropped whatever
    /// account fetch was running — and the account fetch is the one that owns
    /// `loading`, so the spinner under the portrait had nothing left that
    /// could clear it.
    fn spawn_calendar(&mut self, work: impl FnOnce() -> AccountFetch + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.calendar_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    pub fn setup_git(&mut self) {
        match gh::auth_setup_git() {
            Ok(msg) | Err(msg) => {
                self.message = msg;
                self.bump();
            }
        }
    }

    pub fn open_profile(&self) {
        let url = if !self.profile.html_url.is_empty() {
            self.profile.html_url.as_str()
        } else if !self.profile.login.is_empty() {
            return drop(gh::open_in_browser(&format!(
                "https://github.com/{}",
                self.profile.login
            )));
        } else {
            "https://github.com"
        };
        let _ = gh::open_in_browser(url);
    }

    pub fn open_install_docs(&self) {
        let _ = gh::open_gh_install_docs();
    }

    /// Drain login output and any finished background probe. True when the
    /// snapshot the face holds is now stale.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;

        if let Some(session) = self.login.as_mut() {
            if let Some(result) = session.poll() {
                self.login = None;
                match result {
                    Ok(msg) => {
                        self.message = msg;
                        self.spawn_fetch("Loading GitHub profile…");
                    }
                    Err(e) => {
                        self.message = e;
                        self.spawn_fetch("");
                    }
                }
                changed = true;
            } else if !session.code_delivered {
                if let Some(code) = session.code.clone() {
                    let url = session
                        .url
                        .clone()
                        .unwrap_or_else(|| "https://github.com/login/device".into());
                    let _ = copy_code(&code);
                    let _ = gh::open_in_browser(&url);
                    session.code_delivered = true;
                    self.message = format!("Code {code} copied · paste it at github.com/login/device");
                    changed = true;
                }
            }
        }

        if let Some(rx) = self.fetch_rx.as_mut() {
            match rx.try_recv() {
                Ok(fetch) => {
                    self.fetch_rx = None;
                    self.adopt(fetch);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.fetch_rx = None;
                    self.loading = false;
                    self.ever_loaded = true;
                    self.message = "Could not reach GitHub".into();
                    self.bump();
                    changed = true;
                }
            }
        }

        if let Some(rx) = self.calendar_rx.as_mut() {
            match rx.try_recv() {
                Ok(fetch) => {
                    self.calendar_rx = None;
                    self.adopt(fetch);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    // One year's calendar failing is not "could not reach
                    // GitHub" — the profile beside it may be on screen and
                    // perfectly current. It also must not touch `loading`,
                    // which belongs to the account fetch.
                    self.calendar_rx = None;
                }
            }
        }

        changed
    }

    /// Take whatever a fetch thread came back with.
    ///
    /// One place rather than one per receiver: which channel an answer arrived
    /// on says who asked, and nothing about what to do with it.
    fn adopt(&mut self, fetch: AccountFetch) {
        match fetch {
            AccountFetch::Ready {
                info,
                profile,
                contributions,
                message,
            } => {
                self.loading = false;
                self.ever_loaded = true;
                self.info = info;
                self.profile = profile.unwrap_or_default();
                self.contributions = contributions;
                if self.profile.created_year > 0 {
                    self.contrib_year_min = self.profile.created_year;
                }
                if !message.is_empty() {
                    self.message = message;
                } else if self.info.state == GhAuthState::LoggedIn {
                    self.message = self.info.detail.clone();
                }
            }
            AccountFetch::Calendar {
                contributions,
                year,
            } => {
                // A calendar for a year the user has since moved off is an
                // answer to a question that is no longer being asked.
                if year == self.contrib_year {
                    self.contributions = contributions;
                }
            }
        }
        self.bump();
    }

    fn spawn_fetch(&mut self, message: &str) {
        self.spawn_fetch_with(message, None);
    }

    fn spawn_fetch_with(
        &mut self,
        message: &str,
        work: Option<fn() -> AccountFetch>,
    ) {
        self.loading = true;
        self.ever_loaded = true;
        if !message.is_empty() {
            self.message = message.to_string();
        }
        self.bump();
        let (tx, rx) = mpsc::channel();
        self.fetch_rx = Some(rx);
        thread::spawn(move || {
            let result = if let Some(work) = work {
                work()
            } else {
                let info = gh::auth_status();
                let profile = if info.state == GhAuthState::LoggedIn {
                    gh::fetch_profile()
                } else {
                    None
                };
                let contributions = if info.state == GhAuthState::LoggedIn {
                    gh::fetch_contributions(None).unwrap_or_default()
                } else {
                    GhContributions::default()
                };
                AccountFetch::Ready {
                    info,
                    profile,
                    contributions,
                    message: String::new(),
                }
            };
            let _ = tx.send(result);
        });
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

fn copy_code(code: &str) -> bool {
    suisei_core::clipboard::copy(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(account: &mut GitHubAccount) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (account.fetch_rx.is_some() || account.calendar_rx.is_some())
            && std::time::Instant::now() < deadline
        {
            account.poll();
            thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Picking a contribution year must not cancel the account fetch.
    ///
    /// Both used to assign one `fetch_rx`, so the second to start dropped the
    /// first receiver. The dropped thread's `send` failed silently and its
    /// answer never arrived — and for the account fetch that answer is the
    /// only thing that clears `loading`, so the portrait spun for the rest of
    /// the session. The sequence is an ordinary one: the year picker has years
    /// to offer as soon as the profile lands, which is when its fetch is still
    /// finishing the calendar.
    #[test]
    fn asking_for_a_year_does_not_cancel_the_account_fetch() {
        let mut account = GitHubAccount::new();
        account.spawn_fetch_with(
            "Loading…",
            Some(|| AccountFetch::Ready {
                info: GhAuthInfo::default(),
                profile: None,
                contributions: GhContributions::default(),
                message: "loaded".into(),
            }),
        );
        assert!(account.loading, "the account fetch owns the spinner");

        account.contrib_year = 2024;
        account.spawn_calendar(|| AccountFetch::Calendar {
            contributions: GhContributions::default(),
            year: 2024,
        });
        assert!(
            account.fetch_rx.is_some(),
            "the account fetch is still the one being waited on"
        );

        drain(&mut account);
        assert!(!account.loading, "both answers arrived and the spinner stopped");
        assert_eq!(account.message, "loaded");
    }

    /// And the reverse: a refresh while a year is loading does not silently
    /// abandon the calendar.
    #[test]
    fn an_account_fetch_does_not_cancel_a_year() {
        let mut account = GitHubAccount::new();
        account.contrib_year = 2023;
        account.spawn_calendar(|| AccountFetch::Calendar {
            contributions: GhContributions {
                total: 41,
                ..GhContributions::default()
            },
            year: 2023,
        });
        account.spawn_fetch_with(
            "Refreshing…",
            Some(|| AccountFetch::Ready {
                info: GhAuthInfo::default(),
                profile: None,
                contributions: GhContributions::default(),
                message: "refreshed".into(),
            }),
        );
        assert!(account.calendar_rx.is_some());

        drain(&mut account);
        assert!(!account.loading);
    }

    /// A calendar for a year the user has moved off is an answer to a question
    /// nobody is asking any more.
    #[test]
    fn a_stale_year_is_dropped() {
        let mut account = GitHubAccount::new();
        account.contrib_year = 2025;
        account.adopt(AccountFetch::Calendar {
            contributions: GhContributions {
                total: 999,
                ..GhContributions::default()
            },
            year: 2019,
        });
        assert_eq!(account.contributions.total, 0, "not the year on screen");
    }
}
