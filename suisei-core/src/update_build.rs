//! Updating by building the source, and every way it can fail.
//!
//! Suisei is not signed — a Developer ID costs more than this stage of the
//! project can justify — so a downloaded binary would put the receiver through
//! System Settings → Privacy & Security on **every** update. Building the
//! tagged commit on the machine that will run it avoids that entirely: a file
//! written by a local build carries no `com.apple.quarantine`, so Gatekeeper
//! never asks. (Measured: the attribute is set by the DOWNLOADING app —
//! browsers, Mail — and not by the network.)
//!
//! # The rule
//!
//! **There is a working Suisei on disk at every instant.** Not "almost always",
//! and not "unless the power goes out during the swap". Every decision below
//! follows from that one sentence.
//!
//! # How each failure ends
//!
//! | where | what goes wrong | what the user is left with |
//! |---|---|---|
//! | check | no network, no tags, rate limit | the app they have; the page says when the check last succeeded, never "up to date" |
//! | preflight | no cargo, no swiftc, no disk, unwritable app | the app they have, and a sentence naming exactly what is missing — **before anything is downloaded** |
//! | clone | network dies, disk fills | the app they have; the partial clone is deleted |
//! | build | compile error, out of memory, cancelled | the app they have; the build log is kept and named |
//! | stage | disk fills writing the result | the app they have; the half-written bundle is deleted |
//! | swap | power loss mid-rename | the app they have — see below |
//!
//! # Why the swap cannot half-happen
//!
//! The obvious `rm -rf old && mv new old` has a window between the two commands
//! where the user owns no editor. A crash there is unrecoverable without a
//! second copy and a repair path.
//!
//! `renamex_np(RENAME_SWAP)` exchanges two directory entries **atomically**:
//! afterwards the app path holds the new build and the staging path holds the
//! old one. There is no instant in between, so there is nothing to recover
//! from — and the old bundle is still sitting there, which makes rolling back a
//! rename rather than a rebuild.
//!
//! It is also why the swap happens at LAUNCH rather than while the editor is
//! running. The exchange itself is safe under a running process — the running
//! image keeps its inode — but the process would still be the old version, so
//! it has to restart either way. Doing it at startup means the restart is the
//! one the user already performed.

use std::path::{Path, PathBuf};

/// Space for the clone, the build tree, and the staged app.
///
/// Measured on the tree this ships from: `target/release` alone is 1.7 GB, the
/// vendored frameworks 96 MB, and the repository's history 147 MB. Three
/// gigabytes is that with room to finish rather than to fail at the last link.
pub const REQUIRED_BYTES: u64 = 3 * 1024 * 1024 * 1024;

/// Something that stops an update before it starts.
///
/// Every one of these is checked BEFORE the clone. A build that fails twenty
/// minutes in because `swiftc` was never installed is worse than no updater —
/// it spends the user's evening to tell them something knowable at the click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    NoRust,
    NoSwift,
    NotEnoughDisk { free: u64 },
    AppNotWritable { path: PathBuf },
    RunningFromReadOnly { path: PathBuf },
}

impl Blocker {
    /// What the user is told. One sentence on what is wrong, one on the fix.
    pub fn message(&self) -> String {
        match self {
            Blocker::NoRust => "Rust is not installed. Updating builds Suisei \
                 from source, which needs it: install from https://rustup.rs"
                .into(),
            Blocker::NoSwift => "The Swift compiler is not installed. Updating \
                 builds Suisei from source, which needs it: run \
                 xcode-select --install"
                .into(),
            Blocker::NotEnoughDisk { free } => format!(
                "Not enough disk space: {} free, and building needs about {}.",
                gib(*free),
                gib(REQUIRED_BYTES)
            ),
            Blocker::AppNotWritable { path } => format!(
                "Suisei cannot replace itself at {}. Move it somewhere you own \
                 — usually your Applications folder — and try again.",
                path.display()
            ),
            Blocker::RunningFromReadOnly { path } => format!(
                "Suisei is running from a read-only place ({}). Drag it to your \
                 Applications folder first.",
                path.display()
            ),
        }
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

/// What the machine can offer, as one value.
///
/// Separated from the decision below so the POLICY — what blocks an update and
/// in what order the user hears about it — is testable without a machine that
/// happens to be missing Rust.
#[derive(Debug, Clone)]
pub struct Machine {
    pub has_rust: bool,
    pub has_swift: bool,
    pub free_bytes: u64,
    pub app_path: PathBuf,
    pub app_writable: bool,
    pub app_read_only_volume: bool,
}

/// Everything standing between this machine and an update, worst first.
///
/// A list rather than the first hit: someone missing both toolchains should
/// learn that in one go instead of installing Rust to be told about Swift.
pub fn blockers(m: &Machine) -> Vec<Blocker> {
    let mut out = Vec::new();
    // Where the app lives comes first. It is the one that cannot be fixed by
    // installing something, and telling someone to spend twenty minutes
    // building into a bundle that cannot be replaced is the worst order.
    if m.app_read_only_volume {
        out.push(Blocker::RunningFromReadOnly {
            path: m.app_path.clone(),
        });
    } else if !m.app_writable {
        out.push(Blocker::AppNotWritable {
            path: m.app_path.clone(),
        });
    }
    if !m.has_rust {
        out.push(Blocker::NoRust);
    }
    if !m.has_swift {
        out.push(Blocker::NoSwift);
    }
    if m.free_bytes < REQUIRED_BYTES {
        out.push(Blocker::NotEnoughDisk {
            free: m.free_bytes,
        });
    }
    out
}

impl Machine {
    /// Ask this machine. Cheap — four lookups and a `statvfs`.
    pub fn probe(app_path: &Path) -> Machine {
        let volume_read_only = read_only_volume(app_path);
        Machine {
            // `crate::exec`, not `PATH`: a Finder-launched app inherits
            // `/usr/bin:/bin` and cannot see rustup or Homebrew. Reporting
            // "Rust is not installed" to someone who has had it for years is
            // the exact bug `exec` was written to end.
            has_rust: crate::exec::is_available("cargo"),
            has_swift: crate::exec::is_available("swiftc"),
            free_bytes: free_space(app_path),
            app_writable: !volume_read_only && writable(app_path),
            app_read_only_volume: volume_read_only,
            app_path: app_path.to_path_buf(),
        }
    }
}

fn writable(path: &Path) -> bool {
    // The BUNDLE's parent is what a swap renames within, so that is what has to
    // be writable — not the bundle itself. A read-only .app inside a writable
    // Applications folder can still be replaced.
    let parent = path.parent().unwrap_or(Path::new("/"));
    let probe = parent.join(".suisei-update-probe");
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn read_only_volume(path: &Path) -> bool {
    // A .app opened straight out of a mounted .dmg is the case this catches,
    // and it is a common one: people run it from the disk image and never drag
    // it anywhere.
    path.starts_with("/Volumes/") && !writable(path)
}

fn free_space(path: &Path) -> u64 {
    let dir = path.parent().unwrap_or(Path::new("/"));
    let out = std::process::Command::new("/bin/df")
        .arg("-k")
        .arg(dir)
        .output();
    let Ok(out) = out else { return 0 };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|k| k.parse::<u64>().ok())
        .map(|k| k * 1024)
        .unwrap_or(0)
}

/// Where an update's working files live.
///
/// Under Caches on purpose: everything here is reproducible from the tag, so a
/// machine short of space may throw it away and cost the user a rebuild rather
/// than a broken install.
pub fn work_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Caches/Suisei/update")
}

/// The built app waiting for the next launch, for `version`.
pub fn staged_app(version: &str) -> PathBuf {
    work_dir().join(format!("staged-{version}")).join("Suisei.app")
}

/// Where a failed build's output is kept, so "it failed" can be followed up.
pub fn build_log(version: &str) -> PathBuf {
    work_dir().join(format!("build-{version}.log"))
}

/// What the next launch needs to know, written when a build succeeds.
///
/// A three-line text file rather than JSON: it is read at startup before
/// anything else works, a human may have to look at it to understand what the
/// editor is about to do, and there is no version of it that half-parses into
/// something plausible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub version: String,
    pub sha: String,
    pub app: PathBuf,
}

impl Pending {
    pub fn serialise(&self) -> String {
        format!(
            "version={}\nsha={}\napp={}\n",
            self.version,
            self.sha,
            self.app.display()
        )
    }

    /// Anything unexpected reads as "no update pending".
    ///
    /// This file decides whether the editor replaces itself at startup, so a
    /// truncated or hand-edited one must mean "do nothing" — never "do
    /// something with the fields I could make out".
    pub fn parse(text: &str) -> Option<Pending> {
        let mut version = None;
        let mut sha = None;
        let mut app = None;
        for line in text.lines() {
            let (k, v) = line.split_once('=')?;
            match k {
                "version" => version = Some(v.to_string()),
                "sha" => sha = Some(v.to_string()),
                "app" => app = Some(PathBuf::from(v)),
                _ => return None,
            }
        }
        let (version, sha, app) = (version?, sha?, app?);
        if version.is_empty() || sha.is_empty() || app.as_os_str().is_empty() {
            return None;
        }
        Some(Pending { version, sha, app })
    }
}

pub fn pending_path() -> PathBuf {
    work_dir().join("pending")
}

/// The update waiting for this launch, if there is a real one.
pub fn pending_for(current_version: &str) -> Option<Pending> {
    pending_at(&pending_path(), current_version)
}

/// As above, from a named marker.
///
/// The path is a parameter so a test can own one. Reading the real
/// `~/Library/Caches` file would make these tests race each other AND edit the
/// developer's actual install state — the same shared-fixture bug that once
/// made a block-selection test read another test's document.
///
/// Every check here is a way the file can be right and the update still wrong:
/// the staged bundle deleted by a cache sweep, a marker left behind by a
/// version already installed, a bundle that never finished being written.
pub fn pending_at(marker: &Path, current_version: &str) -> Option<Pending> {
    let text = std::fs::read_to_string(marker).ok()?;
    let p = Pending::parse(&text)?;
    if p.version == current_version {
        // Already applied — the marker outlived its update. Not an error.
        return None;
    }
    if !p.app.join("Contents/MacOS/Suisei").is_file() {
        return None;
    }
    Some(p)
}

/// Forget a pending update, whether it was applied or abandoned.
pub fn clear_pending() {
    let _ = std::fs::remove_file(pending_path());
}

/// Exchange two directory entries atomically.
///
/// The whole safety argument of this module rests here. `rm -rf old && mv new
/// old` has a window between the two where the user owns no editor, and a crash
/// in it is unrecoverable. `RENAME_SWAP` has no such instant: before the call
/// the app path holds the old build, after it the new one, and never nothing.
///
/// It leaves the OLD bundle at `staged`, which is not a leftover — it is the
/// rollback, and putting it back is another call to this function.
pub fn swap(staged: &Path, app: &Path) -> Result<(), String> {
    const RENAME_SWAP: libc::c_uint = 0x0000_0002;
    let a = std::ffi::CString::new(staged.as_os_str().as_encoded_bytes())
        .map_err(|_| "staged path is not a C string".to_string())?;
    let b = std::ffi::CString::new(app.as_os_str().as_encoded_bytes())
        .map_err(|_| "app path is not a C string".to_string())?;
    // SAFETY: two valid NUL-terminated paths, and a flag the platform defines.
    let rc = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), RENAME_SWAP) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

/// Where a running build has got to, for the page to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Cloning,
    Building,
    Staging,
    /// Built and waiting for the next launch.
    Ready(String),
    /// Gave up. The string is what the user is told; `log` is where the detail
    /// is, when there is any.
    Failed { message: String, log: Option<PathBuf> },
}

/// One step of the pipeline, as reported to the UI.
pub enum Progress {
    Phase(Phase),
    /// The most recent line of build output — a build takes tens of minutes and
    /// a bar with no words looks identical to a hang.
    Line(String),
}

/// Clone the tag, check it is the commit we were promised, build it, stage it.
///
/// Runs on its own thread; every early return leaves the installed app exactly
/// as it was. The only thing this function can do to the running install is
/// nothing.
pub fn run(
    version: &str,
    sha: &str,
    repo: &str,
    report: &dyn Fn(Progress),
) -> Result<Pending, String> {
    let work = work_dir();
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot make {}: {e}", work.display()))?;
    let src = work.join(format!("src-{version}"));
    let staged_root = work.join(format!("staged-{version}"));
    let log = build_log(version);

    // A previous attempt's leftovers are not a starting point. Half a clone
    // looks enough like a clone that git will try to work with it.
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&staged_root);

    report(Progress::Phase(Phase::Cloning));
    let out = crate::exec::tool("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            &format!("v{version}"),
            repo,
            &src.display().to_string(),
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&src);
        return Err(format!(
            "Could not download the source: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("clone failed")
        ));
    }

    // **The tag must still be the commit the check saw.** A tag can be moved,
    // and between the version the user was shown and the source that arrived
    // there is otherwise nothing connecting the two. This is the only integrity
    // check an unsigned update has, so it is not optional.
    let head = crate::exec::tool("git")
        .args(["-C", &src.display().to_string(), "rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if !sha.is_empty() && head != sha {
        let _ = std::fs::remove_dir_all(&src);
        return Err(format!(
            "The tag v{version} moved while updating (expected {}, found {}).              Nothing was installed.",
            &sha[..sha.len().min(12)],
            &head[..head.len().min(12)]
        ));
    }

    report(Progress::Phase(Phase::Building));
    let script = src.join("scripts/release.sh");
    if !script.is_file() {
        let _ = std::fs::remove_dir_all(&src);
        return Err("That release has no build script — refusing to guess how to build it.".into());
    }
    let built = run_build(&src, &script, &log, report)?;

    report(Progress::Phase(Phase::Staging));
    std::fs::create_dir_all(&staged_root)
        .map_err(|e| format!("cannot make {}: {e}", staged_root.display()))?;
    let dest = staged_root.join("Suisei.app");
    // A move within one volume, so this is a rename and cannot half-copy.
    // Across volumes it would be a copy, which is why the staging directory
    // lives under the same HOME the build tree does.
    std::fs::rename(&built, &dest).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staged_root);
        format!("could not stage the new app: {e}")
    })?;

    // The build tree is the biggest thing here (gigabytes) and is worth nothing
    // once the app is out of it.
    let _ = std::fs::remove_dir_all(&src);

    let pending = Pending {
        version: version.to_string(),
        sha: head,
        app: dest,
    };
    std::fs::write(pending_path(), pending.serialise())
        .map_err(|e| format!("could not record the update: {e}"))?;
    report(Progress::Phase(Phase::Ready(version.to_string())));
    Ok(pending)
}

/// Run the release script, streaming its output, and answer where the .app is.
fn run_build(
    src: &Path,
    script: &Path,
    log: &Path,
    report: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;

    let mut child = crate::exec::tool("bash")
        .arg(script)
        .current_dir(src)
        .env("SUISEI_NO_DMG", "1")
        .env("SUISEI_SKIP_TESTS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the build: {e}"))?;

    let mut file = std::fs::File::create(log).ok();
    // stderr is merged in by the script's own `2>&1` on the noisy parts; what
    // arrives here is the step lines, which is what the user should see.
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(f) = file.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            report(Progress::Line(line));
        }
    }
    let status = child.wait().map_err(|e| format!("build died: {e}"))?;
    if !status.success() {
        return Err(format!(
            "The build failed. What it printed is in {}.",
            log.display()
        ));
    }
    let app = src.join("suisei-app/.build/Suisei.app");
    if !app.join("Contents/MacOS/Suisei").is_file() {
        return Err(format!(
            "The build reported success but produced no app. See {}.",
            log.display()
        ));
    }
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Machine {
        Machine {
            has_rust: true,
            has_swift: true,
            free_bytes: REQUIRED_BYTES * 2,
            app_path: PathBuf::from("/Applications/Suisei.app"),
            app_writable: true,
            app_read_only_volume: false,
        }
    }

    #[test]
    fn a_ready_machine_is_not_blocked() {
        assert!(blockers(&ok()).is_empty());
    }

    #[test]
    fn every_blocker_says_what_to_do_about_it() {
        // A blocker that reports a problem and hands back nothing is worth less
        // than no check at all: it stops the update and leaves the user with
        // nowhere to go.
        let all = [
            Blocker::NoRust,
            Blocker::NoSwift,
            Blocker::NotEnoughDisk { free: 0 },
            Blocker::AppNotWritable {
                path: "/x/Suisei.app".into(),
            },
            Blocker::RunningFromReadOnly {
                path: "/Volumes/Suisei/Suisei.app".into(),
            },
        ];
        for b in all {
            let m = b.message();
            assert!(m.len() > 30, "{b:?} says too little: {m}");
            assert!(
                m.contains("install") || m.contains("Move") || m.contains("Drag")
                    || m.contains("free"),
                "{b:?} names no way out: {m}"
            );
        }
    }

    #[test]
    fn a_missing_toolchain_is_reported_before_anything_is_downloaded() {
        let mut m = ok();
        m.has_rust = false;
        assert_eq!(blockers(&m), vec![Blocker::NoRust]);
    }

    #[test]
    fn both_toolchains_are_reported_together() {
        // Installing Rust only to be told about Swift is two evenings.
        let mut m = ok();
        m.has_rust = false;
        m.has_swift = false;
        assert_eq!(blockers(&m), vec![Blocker::NoRust, Blocker::NoSwift]);
    }

    #[test]
    fn where_the_app_lives_is_the_first_thing_said() {
        // It is the blocker that cannot be fixed by installing something, so
        // hearing it after "go install Rust" would be the wrong order.
        let mut m = ok();
        m.has_rust = false;
        m.app_writable = false;
        assert_eq!(
            blockers(&m).first(),
            Some(&Blocker::AppNotWritable {
                path: PathBuf::from("/Applications/Suisei.app")
            })
        );
    }

    #[test]
    fn a_disk_that_cannot_hold_the_build_blocks_it() {
        let mut m = ok();
        m.free_bytes = REQUIRED_BYTES - 1;
        assert_eq!(
            blockers(&m),
            vec![Blocker::NotEnoughDisk {
                free: REQUIRED_BYTES - 1
            }]
        );
    }

    #[test]
    fn running_from_a_disk_image_is_its_own_message() {
        // "Not writable" would be true and useless. The fix is to drag it out,
        // and that is a different sentence.
        let mut m = ok();
        m.app_read_only_volume = true;
        m.app_writable = false;
        m.app_path = "/Volumes/Suisei 0.1.0/Suisei.app".into();
        let b = blockers(&m);
        assert_eq!(b.len(), 1);
        assert!(matches!(b[0], Blocker::RunningFromReadOnly { .. }));
        assert!(b[0].message().contains("Applications"));
    }

    #[test]
    fn the_working_files_are_throwaway() {
        // Under Caches, because everything here rebuilds from the tag. A
        // machine short of space may delete it and cost a rebuild, never an
        // install.
        assert!(work_dir().to_string_lossy().contains("Library/Caches"));
        assert!(staged_app("0.2.0").starts_with(work_dir()));
        assert!(build_log("0.2.0").starts_with(work_dir()));
    }

    #[test]
    fn two_versions_do_not_share_a_staging_directory() {
        assert_ne!(staged_app("0.2.0"), staged_app("0.3.0"));
        assert_ne!(build_log("0.2.0"), build_log("0.3.0"));
    }
}
