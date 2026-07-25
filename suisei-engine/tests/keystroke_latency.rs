//! End-to-end Rust-side cost of ONE keystroke, on an EMPTY buffer.
//!
//! This exists to settle a boundary question, not to micro-optimise: typing in
//! a brand-new empty document feels slow, and the cost of an empty document is
//! by definition independent of the text engine. This measures everything the
//! Rust side does per key — `dispatch_key` → `recompose` → full scene compose —
//! so whatever is left over is the Swift/SwiftUI side.
//!
//! ```text
//! cargo test -p suisei-engine --release --test keystroke_latency -- --ignored --nocapture
//! ```

use std::time::Instant;

use suisei_engine::bridge::input::key_from_ffi;
use suisei_engine::Engine;

/// FfiKeyCode::Char — see bridge::input.
const CODE_CHAR: u32 = 1;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn report(label: &str, mut samples: Vec<f64>) {
    let n = samples.len();
    let sum: f64 = samples.iter().sum();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{label:<40} n={n:<4} mean={:>8.3}ms  p50={:>8.3}ms  p95={:>8.3}ms  max={:>8.3}ms",
        sum / n as f64,
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        samples[n - 1],
    );
}

fn type_chars(engine: &mut Engine, count: usize) -> Vec<f64> {
    let mut samples = Vec::with_capacity(count);
    for i in 0..count {
        let ch = if i % 10 == 9 { ' ' } else { 'a' };
        let ev = key_from_ffi(CODE_CHAR, ch as u32, 0, 0).expect("key event");
        let t = Instant::now();
        engine.dispatch_key(ev);
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples
}

#[test]
#[ignore = "measurement, not an assertion"]
fn empty_buffer_keystroke_cost() {
    println!();
    println!("=== Rust-side cost of one keystroke (dispatch + recompose + compose) ===");

    // CSS px, line height, cell width, dpr — matching a real Suisei window.
    for (w, h) in [(900.0f32, 600.0f32), (1600.0, 1000.0)] {
        let mut engine = Engine::new();
        engine.resize(w, h, 18.0, 8.0, 2.0);
        // Warm-up: the first keystroke pays one-time setup.
        let _ = type_chars(&mut engine, 5);
        report(
            &format!("empty buffer · window {w:.0}x{h:.0}"),
            type_chars(&mut engine, 60),
        );
    }

    println!();
    println!("If this is well under a frame, the latency the user feels is NOT here —");
    println!("it is in the Swift side (chrome pull + SwiftUI publish + redraw).");
    println!();
}

/// The tick runs at 20 Hz whether or not anyone is typing, so anything O(file)
/// inside it is a permanent tax on every large document — invisible to the
/// keystroke measurements above, which never call it.
#[test]
#[ignore = "measurement, not an assertion"]
fn idle_tick_cost_by_file_size() {
    println!();
    println!("=== cost of ONE idle tick (the face calls this at 20Hz) ===");
    for lines in [200usize, 2_000, 20_000] {
        for dirty in [false, true] {
            let mut engine = Engine::new();
            engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
            engine.app.buffer = suisei_core::buffer::Buffer::from_string(
                &"let x = some_function(argument, another);\n".repeat(lines),
            );
            engine.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_tick_cost.rs"));
            engine.app.modified = dirty;
            for _ in 0..5 {
                engine.tick(50); // warm
            }
            let mut samples = Vec::with_capacity(60);
            for _ in 0..60 {
                let t = Instant::now();
                engine.tick(50);
                samples.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            report(
                &format!("{lines:>6} lines · {}", if dirty { "dirty" } else { "clean" }),
                samples,
            );
        }
    }
    println!();
    println!("A clean buffer should cost near zero: nothing needs the document.");
    println!("If it scales with line count, something is rebuilding the whole text.");
    println!();
}

#[test]
#[ignore = "measurement, not an assertion"]
fn large_buffer_typing_cost() {
    // The regression guard for undo coalescing: `gui_insert_text` snapshots the
    // buffer once per typing RUN, not per keystroke. Without coalescing this
    // clones every line on every character — O(file) per key — and a 6k-line
    // file crawls. With it, typing a run stays flat regardless of file size.
    println!();
    println!("=== keystroke cost while typing into a 6,000-line file ===");
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    for _ in 0..6000 {
        engine.dispatch_key(key_from_ffi(CODE_CHAR, 'x' as u32, 0, 0).unwrap());
        engine.dispatch_key(key_from_ffi(2 /* Enter */, 0, 0, 0).unwrap());
    }
    // Now type a run of characters mid-document and measure per-key cost.
    let _ = type_chars(&mut engine, 5); // warm
    report("6,000-line file · typing run", type_chars(&mut engine, 100));
    println!();
    println!("If this tracks the empty-buffer cost, coalescing is holding; a jump");
    println!("to O(file) per key means a per-keystroke snapshot slipped back in.");
    println!();
}
