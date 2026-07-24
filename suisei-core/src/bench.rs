//! In-editor self-benchmark (`:bench`).
//!
//! Times the hot paths that dominate large-file feel — syntax parsing, fold
//! rebuild/lookup, the per-row token slice the renderer uses, bulk paste, and
//! the whole-buffer text join — against a synthetic large source plus the file
//! you actually have open. Everything runs on throwaway local engine instances
//! so it never disturbs editor state.

use std::time::Instant;

use crate::app::App;
use crate::buffer::Buffer;
use crate::fold::FoldState;
use crate::syntax::SyntaxEngine;

pub struct BenchResult {
    pub name: String,
    pub ms: f64,
    pub detail: String,
}

impl BenchResult {
    fn new(name: &str, ms: f64, detail: String) -> Self {
        Self { name: name.to_string(), ms, detail }
    }
}

pub struct BenchReport {
    pub results: Vec<BenchResult>,
    pub total_ms: f64,
    /// Line count of the synthetic workload the fixed benches ran against.
    pub synthetic_lines: usize,
}

fn time_ms(mut f: impl FnMut()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1000.0
}

/// A pinch of nested-indent Rust so folding and tree-sitter have real work.
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

/// Ops/ms → Mops/s, guarding a near-zero elapsed time.
fn mops(count: usize, ms: f64) -> f64 {
    count as f64 / ms.max(1e-9) / 1000.0
}

fn mb_per_s(bytes: usize, ms: f64) -> f64 {
    (bytes as f64 / 1_048_576.0) / (ms.max(1e-9) / 1000.0)
}

pub fn run(app: &App) -> BenchReport {
    let mut results = Vec::new();

    let src = synthetic_rust(4000);
    let line_vec: Vec<String> = src.split('\n').map(|s| s.to_string()).collect();
    let n_lines = line_vec.len();

    // 1) Tree-sitter parse of the whole synthetic buffer.
    let mut eng = SyntaxEngine::new();
    let ms = time_ms(|| eng.parse(&src, Some("rs")));
    let tokens = eng.tokens.len();
    results.push(BenchResult::new(
        "syntax parse (rust)",
        ms,
        format!("{n_lines} lines · {tokens} tokens · {:.0} klines/s", n_lines as f64 / ms.max(1e-9)),
    ));

    // 2) Indent-fold rebuild.
    let mut folds = FoldState::new();
    let ms = time_ms(|| folds.rebuild(&line_vec, 4));
    let ranges = folds.ranges.len();
    results.push(BenchResult::new(
        "fold rebuild",
        ms,
        format!("{ranges} ranges · {:.0} klines/s", n_lines as f64 / ms.max(1e-9)),
    ));

    // 3) fold_at lookup — the per-row-per-frame call, now O(1).
    let iters = 1_000_000usize;
    let mut hits = 0usize;
    let ms = time_ms(|| {
        for i in 0..iters {
            let row = i.wrapping_mul(2654435761) % n_lines;
            if folds.fold_at(row).is_some() {
                hits += 1;
            }
        }
    });
    results.push(BenchResult::new(
        "fold_at ×1M",
        ms,
        format!("{:.1} Mops/s ({hits} hits)", mops(iters, ms)),
    ));

    // 4) tokens_for_row — the renderer's per-row token slice, now O(log n).
    let reps = (1_000_000 / n_lines.max(1)).max(1);
    let mut counted = 0usize;
    let ms = time_ms(|| {
        for _ in 0..reps {
            for row in 0..n_lines {
                counted += eng.tokens_for_row(row).len();
            }
        }
    });
    let calls = reps * n_lines;
    results.push(BenchResult::new(
        "tokens_for_row (render path)",
        ms,
        format!("{:.1} Mops/s ({calls} calls, {counted} toks)", mops(calls, ms)),
    ));

    // 5) Bulk paste — insert_str of a long single line, now O(n) not O(n²).
    let blob = "x".repeat(200_000);
    let mut buf = Buffer::from_string("seed");
    buf.cursor.col = 4;
    let ms = time_ms(|| buf.insert_str(&blob));
    results.push(BenchResult::new(
        "insert_str 200KB paste",
        ms,
        format!("{:.0} MB/s", mb_per_s(blob.len(), ms)),
    ));

    // 6) Whole-buffer join — runs once per edit before reparse.
    let joined = Buffer::from_string(&src);
    let ms = time_ms(|| {
        let _ = joined.text();
    });
    results.push(BenchResult::new(
        "buffer.text() join",
        ms,
        format!("{:.0} MB/s", mb_per_s(src.len(), ms)),
    ));

    // 7) Your actual open file, for realism.
    let real = app.buffer.text();
    if !real.trim().is_empty() {
        let ext = app.file_extension();
        let mut e2 = SyntaxEngine::new();
        let rows = app.buffer.line_count();
        let ms = time_ms(|| e2.parse(&real, ext.as_deref()));
        let kb = real.len() / 1024;
        let toks = e2.tokens.len();
        results.push(BenchResult::new(
            "your file: parse",
            ms,
            format!("{rows} lines · {kb} KB · {toks} tokens"),
        ));
    }

    let total_ms = results.iter().map(|r| r.ms).sum();
    BenchReport { results, total_ms, synthetic_lines: n_lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_produces_results_without_panicking() {
        let app = App::new();
        let report = run(&app);
        // The six synthetic benches always run (the "your file" one is skipped
        // for an empty buffer).
        assert!(report.results.len() >= 6, "got {} results", report.results.len());
        assert!(report.total_ms >= 0.0);
        for r in &report.results {
            assert!(r.ms >= 0.0, "{} had negative time", r.name);
            assert!(!r.detail.is_empty());
        }
    }

    #[test]
    fn synthetic_source_folds_and_parses() {
        let src = synthetic_rust(200);
        assert!(src.lines().count() >= 200);
        let lines: Vec<String> = src.split('\n').map(|s| s.to_string()).collect();
        let mut folds = crate::fold::FoldState::new();
        folds.rebuild(&lines, 4);
        // Nested-indent template must yield foldable ranges.
        assert!(!folds.ranges.is_empty());
    }
}
