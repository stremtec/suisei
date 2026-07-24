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
