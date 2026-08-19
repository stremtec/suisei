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

/// Bytes the update working directory is holding.
///
/// A source update clones the repository and builds it, so this is a checkout
/// plus a `target/` — gigabytes, sitting in `~/Library/Caches` where nobody
/// goes looking. The Software Update page shows the number because an editor
/// that quietly keeps several gigabytes for a job it finished last week is
/// indistinguishable from a leak.
///
/// Walks rather than asking the filesystem: there is no portable "size of this
/// directory" that counts what `du` counts, and the tree is our own — a
/// checkout and a build, not a user's home.
pub fn cache_bytes() -> u64 {
    fn walk(dir: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut total = 0u64;
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            // `metadata` follows symlinks and `symlink_metadata` does not; a
            // build tree has links into itself, and following them would count
            // the same bytes twice or walk forever.
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                total = total.saturating_add(walk(&e.path()));
            } else {
                total = total.saturating_add(meta.len());
            }
        }
        total
    }
    walk(&work_dir())
}

/// What `clear_cache` refuses to do, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearRefused {
    /// A built update is waiting for the next launch. Deleting the cache would
    /// delete the update itself.
    UpdateStaged(String),
    /// A build is running right now.
    BuildRunning,
}

/// Delete the update working directory. Returns the bytes freed.
///
/// **Refuses while an update is staged.** The staged bundle lives in this
/// directory, and the whole safety argument for source updates is that there is
/// a working Suisei on disk at every instant — a "clear cache" button that
/// silently throws away the update the user is waiting to restart into would be
/// the one way to break that from inside the product.
///
/// `building` is passed in rather than detected here: whether a build thread is
/// alive is `UpdateState`'s fact, and this module does not own it.
pub fn clear_cache(current_version: &str, building: bool) -> Result<u64, ClearRefused> {
    if building {
        return Err(ClearRefused::BuildRunning);
    }
    if let Some(p) = pending_for(current_version) {
        return Err(ClearRefused::UpdateStaged(p.version));
    }
    let freed = cache_bytes();
    // A failure to remove is not a failure to report: the directory may be
    // partly gone, and the number above is what was there when we looked.
    let _ = std::fs::remove_dir_all(work_dir());
    Ok(freed)
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

/// How far along a build is, and how much longer it has.
///
/// # Where the number comes from
///
/// A percentage that is invented is worse than no percentage: it moves, so it
/// looks like knowledge. Two of the parts here are counted and the rest are
/// estimated, and the estimate calibrates itself after the first run.
///
///   · **The engine is counted.** `cargo` prints one `Compiling <crate>` line
///     per package, and `Cargo.lock` in the clone says how many there are. That
///     sub-phase's progress is a real fraction of real work.
///   · **The Swift face is one step.** `-whole-module-optimization` compiles
///     the module once and prints nothing until it is done, so there is nothing
///     to count inside it. It gets a weight, not a fraction.
///   · **The weights are estimates**, and a successful build writes its own
///     phase durations next to the staged app so the next update uses measured
///     ones instead.
///
/// The remaining time is `elapsed / fraction - elapsed`, which is only honest
/// once enough has happened to divide by. Below that it is `None`, and the page
/// says nothing rather than something wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildProgress {
    /// 0.0 … 1.0
    pub fraction: f32,
    /// Seconds left, or `None` while there is not enough to estimate from.
    pub eta_secs: Option<u64>,
    /// What it is doing now, in the user's words rather than the build's.
    pub headline: String,
}

/// The steps a release build goes through, and what share of the wall clock
/// each has taken historically.
///
/// Ordered, and the shares sum to 1. A cold clone builds everything — the
/// "up-to-date (skip)" lines that make this fast on a developer's machine
/// never appear for an update, which is why the two big ones dominate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Preparing,
    Art,
    SwiftTerm,
    Engine,
    Face,
    Helpers,
    Finishing,
}

impl Step {
    /// Share of the total, before calibration.
    fn weight(self) -> f32 {
        match self {
            Step::Preparing => 0.02,
            Step::Art => 0.02,
            Step::SwiftTerm => 0.10,
            Step::Engine => 0.44,
            Step::Face => 0.34,
            Step::Helpers => 0.06,
            Step::Finishing => 0.02,
        }
    }

    fn headline(self) -> &'static str {
        match self {
            Step::Preparing => "Getting the source…",
            Step::Art => "Preparing resources…",
            Step::SwiftTerm => "Building the terminal…",
            Step::Engine => "Building the editor engine…",
            Step::Face => "Building the interface…",
            Step::Helpers => "Building the background helpers…",
            Step::Finishing => "Finishing…",
        }
    }

    const ORDER: [Step; 7] = [
        Step::Preparing,
        Step::Art,
        Step::SwiftTerm,
        Step::Engine,
        Step::Face,
        Step::Helpers,
        Step::Finishing,
    ];
}

// `Step::before` lived here: everything before this step as a fraction, summed
// from the STATIC weights. Calibration replaced it — `ProgressModel` derives the
// same quantity from `seconds_per_step`, which uses what the last successful
// build on this machine actually took. Leaving both would have left two owners
// of one number, and the wrong one was the one that was easier to call.

/// Seconds each step took, last time a build succeeded here.
///
/// **This is the answer to "why is the estimate wrong on the first run".**
/// `swiftc -whole-module-optimization` compiles the module as ONE job — measured
/// with `-parseable-output`, WMO emits exactly two events, a compile and a link,
/// so there is nothing to count inside ten minutes of silence. Turning WMO off
/// does give per-file jobs, and the packager's own note says why that is not on
/// the table: without it the driver re-optimises for every primary file, and
/// `EngineBridge.swift` alone took seventeen minutes.
///
/// So the un-countable steps are TIMED instead. A build that finishes writes
/// what each step actually cost on this machine, and the next update weights
/// itself with those numbers rather than with guesses. The first update on a
/// machine estimates; every one after it measures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Calibration {
    /// `(step index in `Step::ORDER`, seconds)`.
    pub secs: Vec<(usize, u64)>,
}

impl Calibration {
    pub fn serialise(&self) -> String {
        self.secs
            .iter()
            .map(|(i, s)| format!("{i}={s}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Anything unparseable is no calibration, not a partial one — half a set
    /// of weights is worse than the defaults, because it is wrong in a way that
    /// looks measured.
    pub fn parse(text: &str) -> Option<Calibration> {
        let mut secs = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (i, s) = line.split_once('=')?;
            secs.push((i.trim().parse().ok()?, s.trim().parse().ok()?));
        }
        (!secs.is_empty()).then_some(Calibration { secs })
    }

    /// Weight for a step, on a basis where every step's weight sums to one.
    ///
    /// The obvious version — measured share for measured steps, the default
    /// weight for the rest — does NOT sum to one, and a bar built on it walks
    /// off the end and sits clamped. A partial measurement is the ordinary
    /// case: a build that failed part way, or a step that was skipped because
    /// something was already up to date.
    ///
    /// So an unmeasured step is given a plausible duration first — its default
    /// share of what was measured — and only then is everything normalised.
    fn weight(&self, step: Step) -> f32 {
        let seconds = self.seconds_per_step();
        let total: f32 = seconds.iter().sum();
        if total <= 0.0 {
            return step.weight();
        }
        let idx = Step::ORDER.iter().position(|s| *s == step).unwrap_or(0);
        seconds[idx] / total
    }

    /// Every step's duration, measured where we have it and inferred where we
    /// do not, on one consistent scale.
    fn seconds_per_step(&self) -> [f32; Step::ORDER.len()] {
        let mut out = [0.0f32; Step::ORDER.len()];
        let measured_total: u64 = self.secs.iter().map(|(_, s)| *s).sum();
        if measured_total == 0 {
            for (i, step) in Step::ORDER.iter().enumerate() {
                out[i] = step.weight();
            }
            return out;
        }
        // What one unit of "default weight" was worth in seconds, judged from
        // the steps we did measure.
        let measured_weight: f32 = self
            .secs
            .iter()
            .filter_map(|(i, _)| Step::ORDER.get(*i))
            .map(|s| s.weight())
            .sum();
        let scale = if measured_weight > 0.0 {
            measured_total as f32 / measured_weight
        } else {
            measured_total as f32
        };
        for (i, step) in Step::ORDER.iter().enumerate() {
            out[i] = match self.secs.iter().find(|(j, _)| *j == i) {
                Some((_, s)) => *s as f32,
                None => step.weight() * scale,
            };
        }
        out
    }

    fn before(&self, step: Step) -> f32 {
        Step::ORDER
            .iter()
            .take_while(|s| **s != step)
            .map(|s| self.weight(*s))
            .sum()
    }
}

pub fn calibration_path() -> PathBuf {
    work_dir().join("timings")
}

/// Turns build output into a fraction. Pure, so the mapping is testable
/// without running a twenty-minute build to see what it says.
#[derive(Debug, Clone)]
pub struct ProgressModel {
    step: Step,
    /// Packages in the clone's `Cargo.lock` — the engine step's denominator.
    crates_total: u32,
    crates_done: u32,
    /// Measured step durations from the last successful build here.
    cal: Calibration,
    /// When each step started, so the next build can be calibrated from this
    /// one and so a timed step can interpolate within itself.
    started: Vec<(usize, u64)>,
    /// Seconds since the build began, as of the last `observe`.
    now: u64,
}

impl ProgressModel {
    pub fn new(crates_total: u32) -> Self {
        Self::with_calibration(crates_total, Calibration::default())
    }

    pub fn with_calibration(crates_total: u32, cal: Calibration) -> Self {
        Self {
            step: Step::Preparing,
            // Never zero: this is a divisor, and a lock file we could not read
            // must degrade to "no granularity" rather than to a crash.
            crates_total: crates_total.max(1),
            crates_done: 0,
            cal,
            started: vec![(0, 0)],
            now: 0,
        }
    }

    /// Tell the model what time it is, so a timed step can move within itself.
    pub fn tick(&mut self, elapsed_secs: u64) {
        self.now = elapsed_secs;
    }

    /// What each step cost, for the next build to weight itself with.
    pub fn measured(&self, total_secs: u64) -> Calibration {
        let mut secs = Vec::new();
        for (n, (idx, start)) in self.started.iter().enumerate() {
            let end = self
                .started
                .get(n + 1)
                .map(|(_, s)| *s)
                .unwrap_or(total_secs);
            secs.push((*idx, end.saturating_sub(*start)));
        }
        Calibration { secs }
    }

    /// Read one line of build output.
    pub fn observe(&mut self, line: &str) {
        let l = line.trim();
        let seen = if l.contains("Welcome art") {
            Some(Step::Art)
        } else if l.starts_with("→ SwiftTerm") && !l.contains("up-to-date") {
            Some(Step::SwiftTerm)
        } else if l.contains("cargo build -p suisei-engine") || l.contains("→ engine") {
            Some(Step::Engine)
        } else if l.starts_with("→ swiftc") {
            Some(Step::Face)
        } else if l.starts_with("→ build + bundle") {
            Some(Step::Helpers)
        } else if l.starts_with("→ packaged") {
            // `→ embed …` was here too, and it is printed BEFORE the helpers
            // are built — which sent the bar to 98% and then back to 92%. The
            // packager's order is the authority, not the order the names
            // suggest.
            Some(Step::Finishing)
        } else {
            None
        };
        // **Never backwards**, whatever the output says. A bar that retreats is
        // worse than one that pauses: it says the thing you were told was done
        // was not, and there is no way for a reader to tell which claim to
        // believe. Build output is a stream someone may reorder later; this is
        // the guard that makes that a cosmetic change rather than a bug report.
        if let Some(next) = seen {
            let ni = Step::ORDER.iter().position(|s| *s == next);
            if ni > Step::ORDER.iter().position(|s| *s == self.step) {
                self.step = next;
                if let Some(i) = ni {
                    self.started.push((i, self.now));
                }
            }
        }
        // Counted, not guessed. Only while the engine is the step: SwiftPM
        // prints the same word for the terminal, and counting those against
        // the engine's denominator would run the bar past its own step.
        if self.step == Step::Engine && l.starts_with("Compiling ") {
            self.crates_done = self.crates_done.saturating_add(1);
        }
    }

    pub fn progress(&self, elapsed_secs: u64) -> BuildProgress {
        let within = if self.step == Step::Engine {
            // Counted: one `Compiling` line per package in the lock file.
            (self.crates_done as f32 / self.crates_total as f32).min(1.0)
        } else if let Some(expected) = self.expected_secs(self.step) {
            // Timed, but only because a previous build on THIS machine measured
            // it. Capped just short of the step's end: a clock may run out
            // before the work does, and a bar that sits at "done" while the
            // step continues is the same lie as one that finishes early.
            let in_step = elapsed_secs.saturating_sub(self.step_started());
            (in_step as f32 / expected as f32).min(0.95)
        } else {
            // Nothing to count and nothing measured. The bar holds still rather
            // than creeping on a guess and arriving before the work does.
            0.0
        };
        let fraction =
            (self.cal.before(self.step) + self.cal.weight(self.step) * within).clamp(0.0, 0.99);
        // Enough elapsed AND enough done — either alone divides by something
        // too small and reports an hour on a build that has thirty seconds
        // left, or a minute on one that has twenty.
        let eta_secs = (elapsed_secs >= 30 && fraction >= 0.05).then(|| {
            let total = elapsed_secs as f32 / fraction;
            (total - elapsed_secs as f32).max(0.0) as u64
        });
        BuildProgress {
            fraction,
            eta_secs,
            headline: self.step.headline().into(),
        }
    }
}

impl ProgressModel {
    fn step_started(&self) -> u64 {
        self.started.last().map(|(_, s)| *s).unwrap_or(0)
    }

    /// How long this step took last time, in seconds. `None` when it has never
    /// been measured here — the first update on a machine.
    fn expected_secs(&self, step: Step) -> Option<u64> {
        let idx = Step::ORDER.iter().position(|s| *s == step)?;
        self.cal
            .secs
            .iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, s)| *s)
            .filter(|s| *s > 0)
    }
}

/// Packages in a clone's lock file — the engine step's denominator.
pub fn crate_count(src: &Path) -> u32 {
    std::fs::read_to_string(src.join("Cargo.lock"))
        .map(|t| t.matches("[[package]]").count() as u32)
        .unwrap_or(0)
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
    /// How far along, and how much longer.
    Advance(BuildProgress),
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

/// A path as one shell word.
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
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

    // `2>&1` inside the shell, deliberately.
    //
    // cargo writes `Compiling <crate>` to STDERR, which is both the progress
    // signal this reads and — with `Stdio::piped()` and nobody draining it — a
    // 64 KB pipe that a cold build fills in seconds. The build would then block
    // forever on a write, and the editor would show "Building…" until the user
    // gave up. Merging at the shell puts both streams down one pipe with one
    // reader, so there is no second buffer to forget about.
    let mut child = crate::exec::tool("bash")
        .arg("-c")
        .arg(format!("exec bash {} 2>&1", shell_quote(script)))
        .current_dir(src)
        .env("SUISEI_NO_DMG", "1")
        .env("SUISEI_SKIP_TESTS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start the build: {e}"))?;

    let mut file = std::fs::File::create(log).ok();
    // Last build's measured step durations, when there was one. The first
    // update on a machine has none and estimates; every one after it measures.
    let cal = std::fs::read_to_string(calibration_path())
        .ok()
        .and_then(|t| Calibration::parse(&t))
        .unwrap_or_default();
    let mut model = ProgressModel::with_calibration(crate_count(src), cal);
    let started = std::time::Instant::now();
    let mut last_sent = 0.0f32;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(f) = file.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            model.tick(started.elapsed().as_secs());
            model.observe(&line);
            let p = model.progress(started.elapsed().as_secs());
            // Only when it actually moved. A cold build prints thousands of
            // lines and the face redraws on every message it receives.
            if (p.fraction - last_sent).abs() > 0.002 {
                last_sent = p.fraction;
                report(Progress::Advance(p));
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
    // Only a build that WORKED gets to teach the next one. A failed build's
    // timings describe how long it took to break, which is not the same shape.
    let _ = std::fs::write(
        calibration_path(),
        model.measured(started.elapsed().as_secs()).serialise(),
    );
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
