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
    fetch_rx: Option<Receiver<AccountFetch>>,
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
        let (tx, rx) = mpsc::channel();
        self.fetch_rx = Some(rx);
        thread::spawn(move || {
            let year_opt = if year == 0 { None } else { Some(year) };
            let contributions = gh::fetch_contributions(year_opt).unwrap_or_default();
            let _ = tx.send(AccountFetch::Calendar {
                contributions,
                year,
            });
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
                Ok(AccountFetch::Ready {
                    info,
                    profile,
                    contributions,
                    message,
                }) => {
                    self.fetch_rx = None;
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
                    self.bump();
                    changed = true;
                }
                Ok(AccountFetch::Calendar {
                    contributions,
                    year,
                }) => {
                    self.fetch_rx = None;
                    if year == self.contrib_year {
                        self.contributions = contributions;
                    }
                    self.bump();
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

        changed
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
