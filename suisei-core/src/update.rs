//! Version check + self-update.
//!
//! On startup (when `update_check = true`, default) a background thread asks
//! GitHub for the latest release tag — non-blocking, silent on any failure,
//! throttled to one network hit per ~4h via `~/.suisei/update_check` (which
//! caches the found version so throttled launches still banner). When a newer version
//! exists the welcome screen shows a notice and `:update` swaps the running
//! binary in place (download → gunzip → atomic rename over `current_exe`),
//! which works for npm / brew / cargo / curl installs alike.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Re-check at most this often (cached result still banners in between).
const CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Default)]
pub struct UpdateState {
    /// Newer version available (plain semver, no leading `v`).
    pub latest: Option<String>,
    /// The commit `latest` names — what an update clones and builds.
    ///
    /// Carried from the tag lookup rather than resolved again later: the tag
    /// could move between the check and the build, and then the version the
    /// user was shown and the source they got would be different things.
    pub latest_sha: Option<String>,
    /// Release notes for `latest`, when the check returned a body.
    pub notes: String,
    /// A self-update finished this session — restart to load it.
    pub installed: bool,
    pub installing: bool,
    check_rx: Option<Receiver<Option<LatestRelease>>>,
    /// A source build in flight: its progress channel, and the last thing it
    /// said. A build runs for tens of minutes, so "no output" and "hung" have
    /// to look different.
    build_rx: Option<Receiver<crate::update_build::Progress>>,
    pub build_phase: Option<crate::update_build::Phase>,
    pub build_line: String,
    /// How far along, and how much longer. See `update_build::BuildProgress`.
    pub build_progress: Option<crate::update_build::BuildProgress>,
    /// `:update` before any check finished — install as soon as one lands.
    install_after_check: bool,
    install_rx: Option<Receiver<Result<String, String>>>,
}

impl UpdateState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kick off the async latest-version lookup. Throttled launches fall
    /// back to the stamp's cached result so a known update still banners.
    pub fn start_check(&mut self, current: &str) {
        if self.check_rx.is_some() {
            return;
        }
        match throttle_state() {
            Throttle::Ready => self.spawn_check(current),
            Throttle::Wait(cached) => {
                if let Some(v) = cached {
                    if is_suisei_release(&v) && is_newer(&v, current) {
                        self.latest = Some(v);
                    }
                }
            }
        }
    }

    fn spawn_check(&mut self, current: &str) {
        let current = current.to_string();
        let (tx, rx) = mpsc::channel();
        self.check_rx = Some(rx);
        std::thread::spawn(move || {
            let found = fetch_latest();
            write_stamp(found.as_ref().map(|r| r.tag.as_str()));
            let newer = found.filter(|r| is_suisei_release(&r.tag) && is_newer(&r.tag, &current));
            let _ = tx.send(newer);
        });
    }

    /// Settings "Check Now" — ignore the 4h throttle.
    pub fn check_now(&mut self, current: &str) {
        if self.check_rx.is_some() {
            return;
        }
        self.spawn_check(current);
    }

    pub fn is_checking(&self) -> bool {
        self.check_rx.is_some()
    }

    /// `:update` with nothing known yet: force a fresh check (bypasses the
    /// throttle) and install automatically when something newer lands.
    pub fn check_now_and_install(&mut self, current: &str) -> String {
        self.install_after_check = true;
        self.spawn_check(current);
        "⟳ checking for updates…".into()
    }

    /// Start building the tagged release on this machine.
    ///
    /// Returns the blockers instead when there are any — checked here, before
    /// a byte is downloaded, because the alternative is spending someone's
    /// evening to tell them `swiftc` was never installed.
    pub fn start_source_update(
        &mut self,
        app_path: &std::path::Path,
    ) -> Result<(), Vec<crate::update_build::Blocker>> {
        use crate::update_build as ub;
        let blockers = ub::blockers(&ub::Machine::probe(app_path));
        if !blockers.is_empty() {
            return Err(blockers);
        }
        if self.build_rx.is_some() {
            return Ok(());
        }
        let (Some(version), sha) = (self.latest.clone(), self.latest_sha.clone().unwrap_or_default())
        else {
            return Ok(());
        };
        let (tx, rx) = mpsc::channel();
        self.build_rx = Some(rx);
        self.build_phase = Some(ub::Phase::Cloning);
        self.build_line.clear();
        self.build_progress = None;
        std::thread::spawn(move || {
            let send = |p: ub::Progress| {
                let _ = tx.send(p);
            };
            if let Err(message) = ub::run(&version, &sha, REPO_URL, &send) {
                // Every failure lands here, and every one of them leaves the
                // installed app untouched — the swap is the only step that
                // could have changed it, and it has not run.
                let log = ub::build_log(&version);
                send(ub::Progress::Phase(ub::Phase::Failed {
                    message,
                    log: log.is_file().then_some(log),
                }));
            }
        });
        Ok(())
    }

    pub fn is_building(&self) -> bool {
        matches!(
            self.build_phase,
            Some(crate::update_build::Phase::Cloning)
                | Some(crate::update_build::Phase::Building)
                | Some(crate::update_build::Phase::Staging)
        )
    }

    /// Drain the build channel. Cheap enough for the frame tick.
    pub fn poll_build(&mut self) -> bool {
        use crate::update_build::{Phase, Progress};
        let Some(rx) = self.build_rx.as_ref() else {
            return false;
        };
        let mut moved = false;
        loop {
            match rx.try_recv() {
                Ok(Progress::Phase(p)) => {
                    let done = matches!(p, Phase::Ready(_) | Phase::Failed { .. });
                    self.build_phase = Some(p);
                    moved = true;
                    if done {
                        self.build_rx = None;
                        break;
                    }
                }
                Ok(Progress::Line(l)) => {
                    self.build_line = l;
                    moved = true;
                }
                Ok(Progress::Advance(p)) => {
                    self.build_progress = Some(p);
                    moved = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The thread died without reporting. Treat silence as
                    // failure rather than as success — the alternative is an
                    // update that claims to be staged and is not.
                    if self.is_building() {
                        self.build_phase = Some(Phase::Failed {
                            message: "The build stopped without saying why.".into(),
                            log: None,
                        });
                    }
                    self.build_rx = None;
                    moved = true;
                    break;
                }
            }
        }
        moved
    }

    /// Drain background results; returns a status message when one lands.
    pub fn poll(&mut self) -> Option<String> {
        if let Some(rx) = self.check_rx.take() {
            match rx.try_recv() {
                Ok(found) => {
                    match found {
                        Some(release) => {
                            self.latest_sha = Some(release.sha);
                            self.latest = Some(release.tag);
                            self.notes = release.notes;
                        }
                        None => {
                            self.latest = None;
                            self.notes.clear();
                        }
                    }
                    let auto = std::mem::take(&mut self.install_after_check);
                    if self.latest.is_some() {
                        if auto {
                            return Some(self.start_install());
                        }
                        return Some("This is not a valid Suisei release.".into());
                    } else if auto {
                        return Some("Already up to date".into());
                    }
                }
                Err(TryRecvError::Empty) => self.check_rx = Some(rx),
                Err(TryRecvError::Disconnected) => {}
            }
        }
        if let Some(rx) = self.install_rx.take() {
            match rx.try_recv() {
                Ok(Ok(msg)) => {
                    self.installing = false;
                    self.installed = true;
                    self.latest = None;
                    return Some(msg);
                }
                Ok(Err(e)) => {
                    self.installing = false;
                    return Some(format!("update failed: {e}"));
                }
                Err(TryRecvError::Empty) => self.install_rx = Some(rx),
                Err(TryRecvError::Disconnected) => self.installing = false,
            }
        }
        None
    }

    /// Install is gated until Suisei publishes its own signed snapshots.
    /// The old xei 3.x line must never be written over this binary.
    pub fn start_install(&mut self) -> String {
        self.installing = false;
        "This is not a valid Suisei release.".into()
    }
}

/// Historic xei tags (3.0.10, …) are not Suisei releases. A leftover
/// `~/.suisei/update_check` stamp used to offer those as an upgrade.
fn is_suisei_release(tag: &str) -> bool {
    let t = tag.trim_start_matches('v');
    let major = t
        .split('.')
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    major < 1 || t.contains("dev") || t.contains("2026")
}

/// Numeric semver compare on `a.b.c`; returns true when `latest` > `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (l, c) = (parse(latest), parse(current));
    for i in 0..l.len().max(c.len()) {
        let (a, b) = (
            l.get(i).copied().unwrap_or(0),
            c.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    false
}

fn xei_dir() -> PathBuf {
    crate::fs_atomic::state_dir()
}

enum Throttle {
    /// Interval elapsed — hit the network.
    Ready,
    /// Inside the window; carries the cached latest version (if any).
    Wait(Option<String>),
}

/// Stamp format: `suisei2 <unix-ts> [<latest-version>]`.
/// Older unprefixed stamps are the xei 3.x cache and are ignored.
fn throttle_state() -> Throttle {
    let stamp = xei_dir().join("update_check");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Ok(prev) = std::fs::read_to_string(&stamp) {
        let mut parts = prev.split_whitespace();
        if parts.next() == Some("suisei2") {
            if let Some(Ok(ts)) = parts.next().map(|p| p.parse::<u64>()) {
                if now.saturating_sub(ts) < CHECK_INTERVAL.as_secs() {
                    return Throttle::Wait(parts.next().map(|s| s.to_string()));
                }
            }
        }
    }
    Throttle::Ready
}

fn write_stamp(latest: Option<&str>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::create_dir_all(xei_dir());
    let body = match latest {
        Some(v) => format!("suisei2 {now} {v}"),
        None => format!("suisei2 {now}"),
    };
    let _ = std::fs::write(xei_dir().join("update_check"), body);
}

/// The repository a release is a tag in.
pub const REPO_URL: &str = "https://github.com/stremtec/suisei";

pub struct LatestRelease {
    /// Plain semver, no leading `v`.
    pub tag: String,
    /// The COMMIT the tag names — what an update clones and builds.
    pub sha: String,
    /// Empty from a tag. A tag carries no body; the notes the old REST path
    /// showed came from a GitHub Release, which is the thing we stopped using.
    pub notes: String,
}

/// The newest release tag in the repository, and the commit it points at.
///
/// **`git ls-remote`, not `api.github.com`.** Measured while designing this:
/// the REST endpoint the old check used answered `403` with `0/60` remaining.
/// Unauthenticated GitHub REST is sixty requests an hour **per IP**, and every
/// other tool on that address spends from the same bucket — so on an office,
/// campus or CGNAT connection the update check is simply broken most of the
/// time, silently, because a failed check looks exactly like "no update". The
/// git protocol has no such limit, over the same TLS to the same host.
///
/// It also asks a better question. A Release is an artifact to download; a tag
/// is a NAME FOR A COMMIT, which is what a source update actually needs.
fn fetch_latest() -> Option<LatestRelease> {
    let out = crate::exec::tool("git")
        .args(["ls-remote", "--tags", REPO_URL])
        // A machine with no network should fail fast, not hang the thread for
        // git's own (much longer) default.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
        .env("GIT_HTTP_LOW_SPEED_TIME", "10")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_ls_remote(&String::from_utf8_lossy(&out.stdout))
}

/// Pick the highest release tag out of `git ls-remote --tags` output.
///
/// Two things this has to get right, and both are invisible until they break:
///
///   · **An annotated tag prints TWICE.** `refs/tags/v1.2.0` carries the tag
///     OBJECT's sha, and `refs/tags/v1.2.0^{}` carries the commit it points at.
///     Cloning the first one checks out nothing — it is not a commit. The
///     dereferenced line wins wherever both appear.
///
///   · **Highest, not last.** Refs come back in lexical order, where `v1.10.0`
///     sorts before `v1.9.0`. Taking the final line would walk the version
///     backwards on exactly the release that matters.
pub fn parse_ls_remote_for_test(text: &str) -> Option<LatestRelease> {
    parse_ls_remote(text)
}

fn parse_ls_remote(text: &str) -> Option<LatestRelease> {
    let mut best: Option<(Vec<u64>, String, String)> = None;
    for line in text.lines() {
        let (sha, refname) = line.split_once('\t')?;
        let Some(name) = refname.strip_prefix("refs/tags/") else {
            continue;
        };
        let (name, dereferenced) = match name.strip_suffix("^{}") {
            Some(n) => (n, true),
            None => (name, false),
        };
        let tag = name.trim_start_matches('v').to_string();
        if !looks_like_version(&tag) || !is_suisei_release(&tag) {
            continue;
        }
        let key = version_key(&tag);
        match &mut best {
            // Same tag seen again: only the dereferenced line may replace it,
            // and it always should — that one is the commit.
            Some((k, t, s)) if *t == tag => {
                if dereferenced {
                    *s = sha.to_string();
                }
                let _ = k;
            }
            Some((k, _, _)) if *k >= key => {}
            _ => best = Some((key, tag, sha.to_string())),
        }
    }
    best.map(|(_, tag, sha)| LatestRelease {
        tag,
        sha,
        notes: String::new(),
    })
}

/// Whether a tag names a version at all.
///
/// Repositories collect tags that are not releases — `nightly`, `latest`,
/// `ci-run-4`. Those parse to version `[0]` and merely LOSE the comparison,
/// which is safe today and quietly wrong: a repository whose only tag is
/// `nightly` would have it treated as the newest release rather than as no
/// release at all. Requiring `<digits>.<digits>` up front says what is meant.
fn looks_like_version(tag: &str) -> bool {
    let mut parts = tag.split('.');
    let (Some(major), Some(minor)) = (parts.next(), parts.next()) else {
        return false;
    };
    !major.is_empty()
        && major.chars().all(|c| c.is_ascii_digit())
        && minor.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// `1.10.2` → `[1, 10, 2]`, for comparing as numbers rather than as text.
fn version_key(tag: &str) -> Vec<u64> {
    tag.split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

// `install_binary` lived here: download a `.gz`, gunzip it, and `mv` the result
// over the running executable. It was dead — `#[allow(dead_code)]`, no callers —
// and it described an update model this product cannot use.
//
// Suisei ships as a `.app` in a `.dmg`. Replacing `Contents/MacOS/Suisei` alone
// would leave `Frameworks/`, `Helpers/` and `Resources/` at the old version, and
// it would break the bundle's signature — ad-hoc today, Developer ID later, and
// invalid either way once the executable no longer matches what was sealed. It
// also named an artifact the release script does not build
// (`suisei-<triple>.gz`; `scripts/release.sh` produces `Suisei-<version>.dmg`).
//
// Deleted rather than left for later, because dead code that looks finished is
// an invitation to wire it up. Updating here means downloading the .dmg and
// replacing the app, and the Software Update page says exactly that.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare() {
        assert!(is_newer("3.0.2", "3.0.1"));
        assert!(is_newer("3.1.0", "3.0.9"));
        assert!(is_newer("4.0.0", "3.9.9"));
        assert!(!is_newer("3.0.1", "3.0.1"));
        assert!(!is_newer("3.0.0", "3.0.1"));
        assert!(is_newer("v3.0.2", "3.0.1"));
        // extra components / junk tolerated
        assert!(is_newer("3.0.1.1", "3.0.1"));
        assert!(!is_newer("garbage", "3.0.1"));
    }

    #[test]
    fn xei_three_is_not_a_suisei_release() {
        assert!(!is_suisei_release("3.0.10"));
        assert!(!is_suisei_release("v3.0.10"));
        assert!(is_suisei_release("0.1.0"));
        assert!(is_suisei_release("2026dev416ad08"));
    }
}
