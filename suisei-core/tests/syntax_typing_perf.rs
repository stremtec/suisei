//! Per-keystroke syntax cost — the number that decides whether typing feels
//! instant in the GUI. Ignored by default (it is a measurement, not an
//! assertion); run it explicitly:
//!
//! ```text
//! cargo test -p suisei-core --release --test syntax_typing_perf -- --ignored --nocapture
//! ```

use std::time::Instant;

use suisei_core::syntax::SyntaxEngine;

/// Nested-indent Rust so tree-sitter has realistic work (mirrors bench.rs).
fn synthetic_rust(lines_target: usize) -> String {
    let mut s = String::with_capacity(lines_target * 40);
    let mut produced = 0usize;
    let mut i = 0usize;
    while produced < lines_target {
        s.push_str(&format!("fn compute_{i}(x: usize) -> usize {{\n"));
        s.push_str("    let mut total: usize = 0; // running sum\n");
        s.push_str("    for step in 0..x {\n");
        s.push_str("        total += step * 2 + 1;\n");
        s.push_str("        if total > 100 { total -= 50; }\n");
        s.push_str("    }\n");
        s.push_str("    total\n");
        s.push_str("}\n");
        s.push('\n');
        produced += 9;
        i += 1;
    }
    s
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

/// Simulate typing: each keystroke inserts one char, then the engine re-parses
/// exactly the way `recompose()` does (full text in, tokens out).
fn measure_typing(lines: usize, keystrokes: usize, at_end: bool) -> Vec<f64> {
    measure_typing_windowed(lines, keystrokes, at_end, None)
}

/// `window` mirrors what the engine passes: viewport rows + overscan.
fn measure_typing_windowed(
    lines: usize,
    keystrokes: usize,
    at_end: bool,
    window: Option<std::ops::Range<usize>>,
) -> Vec<f64> {
    let base = synthetic_rust(lines);
    let mut eng = SyntaxEngine::new();
    // Warm: first parse is cold-cache and not representative of typing.
    eng.parse_window(&base, Some("rs"), window.clone());

    let mut text = base;
    // Type inside a function body in the middle of the file (worst realistic
    // case for a parser: the edit invalidates an enclosing node), or append.
    let insert_at = if at_end {
        text.len()
    } else {
        let mid = text.len() / 2;
        text[..mid].rfind('\n').map(|i| i + 1).unwrap_or(mid)
    };

    let mut samples = Vec::with_capacity(keystrokes);
    for k in 0..keystrokes {
        let ch = if k % 8 == 7 { ' ' } else { 'a' };
        text.insert(insert_at + k, ch);
        let t = Instant::now();
        eng.parse_window(&text, Some("rs"), window.clone());
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples
}

fn report(label: &str, mut samples: Vec<f64>) {
    let n = samples.len();
    let sum: f64 = samples.iter().sum();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{label:<34} n={n:<4} mean={:>8.3}ms  p50={:>8.3}ms  p95={:>8.3}ms  max={:>8.3}ms",
        sum / n as f64,
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        samples[n - 1],
    );
}

#[test]
#[ignore = "measurement, not an assertion"]
fn per_keystroke_syntax_cost() {
    println!();
    println!("=== per-keystroke syntax cost (what the user feels while typing) ===");
    for lines in [500usize, 1500, 3000, 6000] {
        report(
            &format!("{lines} lines · edit mid-file"),
            measure_typing(lines, 40, false),
        );
    }
    report("3000 lines · append at end", measure_typing(3000, 40, true));
    println!();
    println!("--- as the engine actually calls it: viewport window + 400 overscan ---");
    for lines in [3000usize, 6000, 20000] {
        // Editing mid-file, so the window is centred there.
        let mid = lines / 2;
        let win = mid.saturating_sub(400)..(mid + 60 + 400);
        report(
            &format!("{lines} lines · windowed"),
            measure_typing_windowed(lines, 40, false, Some(win)),
        );
    }
    println!();
    println!("Budget: one 60fps frame = 16.7ms, and this runs BEFORE layout/paint.");
    println!();
}
