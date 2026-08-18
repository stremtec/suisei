//! What a keystroke costs on the MAIN thread, which is the thread the user is
//! waiting on.
//!
//! `syntax_typing_perf` in suisei-core measures the PARSE, and the parse runs
//! on the syntax worker — it is not what makes typing feel slow. This drives
//! the two entry points the face's keystrokes reach, `gui_type_char` and
//! `dispatch_key`, through the real engine with a real file open, and times
//! the whole thing. Called on `Engine` rather than through the C shims, which
//! are a null check and a deref around exactly these — and calling `Engine`
//! directly is what lets the warm-up be `flush_syntax` instead of a sleep.
//!
//! ```text
//! cargo test -p suisei-engine --release --test typing_latency -- --ignored --nocapture
//! ```
//!
//! The measurement used to answer a question nobody asked, in four ways:
//!
//! * It typed into a **cold** engine. The worker had not parsed yet, so the
//!   run measured an editor with no tree — the one state the user never types
//!   in for more than a moment.
//! * Its "Return" was `gui_type_char('\n')`, which is not what Return does.
//!   Return is a key, it goes through `dispatch_key`, and smart Return —
//!   copying indentation, opening a body between braces — is the part with any
//!   cost in it. The row measured the fast path being handed a newline.
//! * It stopped at 6,000 lines. Both regressions the performance audit found
//!   were quoted at 20,000: a completion scope walk at 28.5 ms per key, and a
//!   bracket match that cloned the buffer at 43.6 ms.
//! * It asserted nothing, so it could not fail. A benchmark that cannot fail
//!   is documentation.
//!
//! The table is still `#[ignore]`d — a timing table is a thing you read, not a
//! thing that passes. `typing_cost_does_not_scale_with_the_file` runs with
//! every other test and is the part that can fail; see its comment for why it
//! asserts a ratio rather than a millisecond count.

/// Milliseconds of CPU this THREAD has actually burned.
///
/// Not wall clock, and that is the whole point. The assertion below is a ratio
/// of two durations, so anything that steals the CPU inflates the numerator and
/// the denominator unevenly and reads exactly like an O(file) regression.
/// Measured on a healthy tree: 2.9x idle, 4.7x while six cargo builds ran —
/// passing and failing on identical code. And the normal way to run this is
/// `cargo test`, which runs the whole suite in parallel, i.e. always loaded.
///
/// Thread CPU time counts work done rather than time passed, so a busy machine
/// makes it slower to FINISH without changing what it reports.
fn cpu_ms() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, initialised timespec and the clock id is a
    // constant the platform defines.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1_000_000.0
}

use suisei_engine::Engine;
use suisei_engine::bridge::input::key_from_ffi;

/// FfiKeyCode::Enter — see bridge::input.
const CODE_ENTER: u32 = 2;

fn synthetic_rust(lines_target: usize) -> String {
    let mut s = String::with_capacity(lines_target * 40);
    let mut i = 0usize;
    // Counted as we go. The loop condition used to be `s.lines().count()`,
    // which rescans the whole string every iteration — quadratic, and the
    // reason nobody had run this at a size where it says anything.
    let mut lines = 0usize;
    while lines < lines_target {
        s.push_str(&format!("fn compute_{i}(x: usize) -> usize {{\n"));
        s.push_str("    let mut total: usize = 0; // running sum\n");
        s.push_str("    for step in 0..x {\n");
        s.push_str("        total += step * 2 + 1;\n");
        s.push_str("        if total > 100 { total -= 50; }\n");
        s.push_str("    }\n");
        s.push_str("    total\n");
        s.push_str("}\n\n");
        lines += 8;
        i += 1;
    }
    s
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// The typing cost with the worker snapshot taken out, per sample.
fn paired_median(c: &Cost) -> f64 {
    median(
        c.per_key
            .iter()
            .zip(&c.snapshot)
            .map(|(key, snap)| key - snap)
            .collect(),
    )
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    percentile(&samples, 0.50)
}

fn report(label: &str, mut samples: Vec<f64>) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    println!(
        "{label:38} n={:<4} mean={mean:8.3}ms  p50={:8.3}ms  p95={:8.3}ms  max={:8.3}ms",
        samples.len(),
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        samples[samples.len() - 1],
    );
}

/// What to press.
#[derive(Clone, Copy, PartialEq)]
enum Keys {
    /// Plain characters only.
    Plain,
    /// Every eighth key is a real Return.
    WithReturns,
}

/// What one keystroke cost, and how much of that was the worker's snapshot.
struct Cost {
    per_key: Vec<f64>,
    /// `buffer.text()` — the full-document copy `refresh_syntax` hands the
    /// syntax worker once per buffer version, which is once per keystroke.
    snapshot: Vec<f64>,
}

/// One engine with `lines` of Rust open, caret parked mid-file, tree warm.
fn measure(lines: usize, keys: usize, mode: Keys) -> Cost {
    let dir = std::env::temp_dir().join(format!("suisei-typing-{lines}-{}", mode == Keys::Plain));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("big.rs");
    std::fs::write(&path, synthetic_rust(lines)).unwrap();

    let mut engine = Engine::new();
    // A viewport, so the highlight window is a window. The engine queries
    // tokens for the rows it can show, and with no size it can show none.
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());

    // Park the caret in the middle: the top of a file is the cheap case and
    // not the one being complained about.
    let mid = lines / 2;
    let at = suisei_core::buffer::Position { row: mid, col: 0 };
    engine.app.buffer.cursor = at;
    engine.app.sel = suisei_core::selection::SelectionSet::single(
        suisei_core::selection::Selection::caret(at),
    );
    engine.app.scroll = mid.saturating_sub(10);

    // WAIT FOR THE TREE. The worker parses off the main thread, and until its
    // first frame lands the engine is highlighting nothing — cheaper than the
    // steady state, and not a state anyone types in for more than a moment.
    // `flush_syntax` is the same warm-up the colour tests use, and it panics
    // rather than quietly measuring a cold engine.
    engine.flush_syntax();

    // INTERLEAVED, and that is not a detail. What the assertion below actually
    // looks at is `median(per_key) - median(snapshot)`, and at 20,000 lines the
    // snapshot is ~87% of the keystroke — so a three-percent difference in the
    // conditions the two are measured under moves the residual by a quarter.
    // Measured in two consecutive loops, the second one ran on a warmer cache
    // against a buffer that had stopped changing, and the residual swung by
    // 2x between builds that typed at exactly the same speed. Paired samples
    // share the conditions, so what is left is the difference itself.
    let mut out = Vec::with_capacity(keys);
    let mut snapshot = Vec::with_capacity(keys);
    for k in 0..keys {
        let returns = mode == Keys::WithReturns && k % 8 == 7;
        let t = cpu_ms();
        if returns {
            // The real Return. `gui_type_char('\n')` reaches the typing fast
            // path with a newline in it and skips smart Return entirely, so it
            // measured neither what Return does nor what it costs.
            engine.dispatch_key(key_from_ffi(CODE_ENTER, 0, 0, 0).expect("Enter"));
        } else {
            engine.gui_type_char('x');
        }
        out.push(cpu_ms() - t);

        let t = cpu_ms();
        let text = engine.app.buffer.text();
        snapshot.push(cpu_ms() - t);
        drop(text);
    }

    let _ = std::fs::remove_dir_all(&dir);
    Cost {
        per_key: out,
        snapshot,
    }
}

#[test]
#[ignore = "measurement, not an assertion"]
fn per_keystroke_main_thread_cost() {
    println!();
    println!("=== per-keystroke MAIN THREAD cost (warm tree, real Return) ===");
    for lines in [500usize, 1500, 3000, 6000, 20000] {
        let cost = measure(lines, 60, Keys::Plain);
        report(&format!("{lines} lines · plain typing"), cost.per_key);
        report(&format!("{lines} lines ·   of which snapshot"), cost.snapshot);
    }
    let ret = measure(20000, 60, Keys::WithReturns);
    report("20000 lines · every 8th key is Return", ret.per_key);
    println!();
    println!("Budget: one 120fps frame is 8.3ms, and layout and paint come after this.");
    println!();
}

/// A keystroke's cost must not follow the size of the file — beyond the one
/// place it deliberately does.
///
/// A ratio rather than a millisecond ceiling. This runs unoptimised in the
/// ordinary `cargo test` pass, on whatever machine happens to be running it,
/// beside other tests competing for the same cores; an absolute budget would
/// have to be set so loose that it caught nothing, and the number that would
/// catch something is the number that makes the suite flaky.
///
/// **The snapshot is subtracted, and that is the interesting part.**
/// `refresh_syntax` hands the syntax worker a full text snapshot per buffer
/// version, which is once per keystroke, and that copy is O(file) on purpose:
/// the parse is asynchronous, so what it parses has to be a snapshot that will
/// not move underneath it. At 20,000 lines it is 1.5 ms of the 1.9 ms a key
/// costs here — so a raw large-versus-small ratio measures that copy and
/// almost nothing else, and would go on passing while a genuine O(file) pass
/// was added next to it.
///
/// What is left is what this pins. Every regression the performance audit
/// found had the same shape — a per-keystroke pass over the whole document: a
/// completion scope walk, a bracket match that cloned the buffer, and (found
/// by this test, once it was fixed enough to say anything) a scope walk that
/// built a `ScopeSymbol` for all 2,500 of a file's globals to show eight.
/// Ten times the file gives about ten times the cost when one of those is
/// present, and about the same cost when none is.
#[test]
fn typing_cost_does_not_scale_with_the_file() {
    let small = measure(2_000, 40, Keys::Plain);
    let large = measure(20_000, 40, Keys::Plain);

    // The median of the PAIRED differences, not the difference of two medians.
    //
    // The loop above interleaves the samples precisely so each keystroke and
    // the snapshot after it share their conditions — and then this threw the
    // pairing away. At 20,000 lines the snapshot is 87% of the keystroke, so
    // the residual is 13%, and any systematic offset between the two sample
    // sets (the snapshot allocates 600 KB and frees it, so the next keystroke
    // meets a different allocator) lands entirely in that 13% — amplified
    // sevenfold. Measured: the same build reported 0.230 ms here and 0.489 ms
    // in the assertion, and a change that made typing FASTER (1.949 → 1.903 ms
    // per key) failed it 5 runs out of 5.
    //
    // Subtracting per sample cancels the shared conditions where they are
    // actually shared, which is within one iteration.
    let small_ms = paired_median(&small);
    let large_ms = paired_median(&large);

    // Both are well under a tenth of a millisecond when nothing is wrong, and
    // a ratio of two numbers that small is mostly timer noise. The floor keeps
    // the comparison meaningful without weakening it: anything O(file) at
    // 20,000 lines lands far above it.
    let floor = 0.05_f64;
    let ratio = large_ms.max(floor) / small_ms.max(floor);
    // **Re-run on an idle machine before believing a failure.** This measures
    // wall-clock, so a saturated CPU inflates the ratio rather than the
    // constant, which reads exactly like an O(file) regression. Measured: a
    // healthy tree reports 2.93–3.08 idle and 4.5–4.7 while six full cargo
    // rebuilds were running — comfortably passing and clearly failing, same
    // code. That cost an hour and a bisect once; it does not need to again.
    assert!(
        ratio < 4.0,
        "a keystroke got {ratio:.1}x more expensive for a 10x bigger file \
         ({small_ms:.3}ms at 2k lines, {large_ms:.3}ms at 20k, both with the \
         worker snapshot taken out) — something is walking the whole document \
         on every key again"
    );
}
