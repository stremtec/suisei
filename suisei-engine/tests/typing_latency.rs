//! What a keystroke costs on the MAIN thread, which is the thread the user is
//! waiting on.
//!
//! `syntax_typing_perf` in suisei-core measures the PARSE, and the parse runs
//! on the syntax worker — it is not what makes typing feel slow. This drives
//! the real entry point the face calls, `gui_type_char`, through the real
//! engine with a real file open, and times the whole thing.
//!
//! ```text
//! cargo test -p suisei-engine --release --test typing_latency -- --ignored --nocapture
//! ```

use std::ffi::CString;
use std::time::Instant;

fn synthetic_rust(lines_target: usize) -> String {
    let mut s = String::with_capacity(lines_target * 40);
    let mut i = 0usize;
    while s.lines().count() < lines_target {
        s.push_str(&format!("fn compute_{i}(x: usize) -> usize {{\n"));
        s.push_str("    let mut total: usize = 0; // running sum\n");
        s.push_str("    for step in 0..x {\n");
        s.push_str("        total += step * 2 + 1;\n");
        s.push_str("        if total > 100 { total -= 50; }\n");
        s.push_str("    }\n");
        s.push_str("    total\n");
        s.push_str("}\n\n");
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

/// One engine with `lines` of Rust open, caret parked mid-file.
fn measure(lines: usize, keys: usize, newlines: bool) -> Vec<f64> {
    let dir = std::env::temp_dir().join(format!("suisei-typing-{lines}-{newlines}"));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("big.rs");
    std::fs::write(&path, synthetic_rust(lines)).unwrap();

    let e = suisei_engine::ffi::suisei_engine_new();
    assert!(!e.is_null());
    let c = CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(
        suisei_engine::ffi::suisei_engine_open_path(e, c.as_ptr()),
        1,
        "opened the file"
    );

    // Park the caret in the middle: the top of a file is the cheap case and
    // not the one being complained about.
    let mid = (lines / 2) as u32;
    suisei_engine::ffi::suisei_engine_scroll_to(e, mid, 0);
    suisei_engine::ffi::suisei_engine_click_utf16(e, mid, 0, 0);
    suisei_engine::ffi::suisei_engine_gui_ensure_insert(e);

    let mut out = Vec::with_capacity(keys);
    for k in 0..keys {
        let ch: u32 = if newlines && k % 8 == 7 {
            '\n' as u32
        } else {
            'x' as u32
        };
        let t = Instant::now();
        suisei_engine::ffi::suisei_engine_gui_type_char(e, ch);
        out.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    suisei_engine::ffi::suisei_engine_free(e);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
#[ignore = "measurement, not an assertion"]
fn per_keystroke_main_thread_cost() {
    println!();
    println!("=== per-keystroke MAIN THREAD cost (gui_type_char end to end) ===");
    for lines in [500usize, 1500, 3000, 6000] {
        report(&format!("{lines} lines · plain typing"), measure(lines, 60, false));
    }
    report("6000 lines · every 8th key is Return", measure(6000, 60, true));
    println!();
    println!("Budget: one 60fps frame is 16.7ms, and layout and paint come after this.");
    println!();
}
