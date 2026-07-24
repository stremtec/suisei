//! Version check + self-update.
//!
//! On startup (when `update_check = true`, default) a background thread asks
//! GitHub for the latest release tag — non-blocking, silent on any failure,
//! throttled to one network hit per ~4h via `~/.xei/update_check` (which
//! caches the found version so throttled launches still banner). When a newer version
//! exists the welcome screen shows a notice and `:update` swaps the running
//! binary in place (download → gunzip → atomic rename over `current_exe`),
//! which works for npm / brew / cargo / curl installs alike.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Re-check at most this often (cached result still banners in between).
const CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Default)]
pub struct UpdateState {
    /// Newer version available (plain semver, no leading `v`).
    pub latest: Option<String>,
    /// A self-update finished this session — restart to load it.
    pub installed: bool,
    pub installing: bool,
    check_rx: Option<Receiver<Option<String>>>,
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
                    if is_newer(&v, current) {
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
            write_stamp(found.as_deref());
            let newer = found.filter(|v| is_newer(v, &current));
            let _ = tx.send(newer);
        });
    }

    /// `:update` with nothing known yet: force a fresh check (bypasses the
    /// throttle) and install automatically when something newer lands.
    pub fn check_now_and_install(&mut self, current: &str) -> String {
        self.install_after_check = true;
        self.spawn_check(current);
        "⟳ checking for updates…".into()
    }

    /// Drain background results; returns a status message when one lands.
    pub fn poll(&mut self) -> Option<String> {
        if let Some(rx) = self.check_rx.take() {
            match rx.try_recv() {
                Ok(found) => {
                    self.latest = found;
                    let auto = std::mem::take(&mut self.install_after_check);
                    if self.latest.is_some() {
                        if auto {
                            return Some(self.start_install());
                        }
                        let v = self.latest.as_deref().unwrap_or_default();
                        return Some(format!(
                            "⬆ xei v{v} available — :update to install"
                        ));
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

    /// `:update` — replace the running binary with the latest release build.
    pub fn start_install(&mut self) -> String {
        let Some(latest) = self.latest.clone() else {
            return "Already up to date".into();
        };
        if self.installing {
            return "Update already running…".into();
        }
        let Some(triple) = release_triple() else {
            return format!(
                "Self-update unsupported on this platform — run: npm i -g xei-editor (or brew upgrade xei) for v{latest}"
            );
        };
        let Ok(exe) = std::env::current_exe() else {
            return "update: cannot locate current executable".into();
        };
        self.installing = true;
        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        let msg = format!("⬇ downloading v{latest}…");
        std::thread::spawn(move || {
            let _ = tx.send(install_binary(&latest, triple, exe));
        });
        msg
    }
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
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".xei")
}

enum Throttle {
    /// Interval elapsed — hit the network.
    Ready,
    /// Inside the window; carries the cached latest version (if any).
    Wait(Option<String>),
}

/// Stamp format: `<unix-ts> [<latest-version>]`.
fn throttle_state() -> Throttle {
    let stamp = xei_dir().join("update_check");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Ok(prev) = std::fs::read_to_string(&stamp) {
        let mut parts = prev.split_whitespace();
        if let Some(Ok(ts)) = parts.next().map(|p| p.parse::<u64>()) {
            if now.saturating_sub(ts) < CHECK_INTERVAL.as_secs() {
                return Throttle::Wait(parts.next().map(|s| s.to_string()));
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
        Some(v) => format!("{now} {v}"),
        None => now.to_string(),
    };
    let _ = std::fs::write(xei_dir().join("update_check"), body);
}

/// Latest release tag from GitHub (regardless of comparison).
fn fetch_latest() -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "5",
            "-H",
            "User-Agent: xei-update-check",
            "https://api.github.com/repos/stremtec/xei/releases/latest",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = v.get("tag_name")?.as_str()?;
    Some(tag.trim_start_matches('v').to_string())
}

/// Release asset triple for the running platform (self-update targets).
fn release_triple() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("aarch64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-gnu")
    } else {
        // Windows can't replace a running .exe in place — installer path there.
        None
    }
}

/// Download + gunzip + atomic rename over the running executable.
fn install_binary(latest: &str, triple: &str, exe: PathBuf) -> Result<String, String> {
    let url = format!(
        "https://github.com/stremtec/xei/releases/download/v{latest}/xei-{triple}.gz"
    );
    let tmp = exe.with_extension(format!("update-{latest}"));
    let tmp_s = tmp.display().to_string();
    let exe_s = exe.display().to_string();
    let script = format!(
        "curl -fsSL --max-time 120 '{url}' | gunzip > '{tmp_s}' && chmod +x '{tmp_s}' && mv '{tmp_s}' '{exe_s}'"
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(format!("✓ updated to v{latest} — restart xei to use it"))
    } else {
        let _ = std::fs::remove_file(&tmp);
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .next()
            .unwrap_or("download failed (permissions? network?)")
            .to_string())
    }
}

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
}
