//! Does the engine get slower as tabs and splits accumulate?
//!
//! The reported symptom is "the more tabs are open and the more the editor is
//! split, the worse the lag". `tick_breakdown` and `keystroke_latency` already
//! show the tick is flat in *file size*, so this asks the other two questions
//! directly — and specifically at the FFI boundary, because that is where the
//! fixed-size snapshot structs live:
//!
//! - `suisei_engine_chrome` memsets its whole snapshot per call (`ffi.rs`) —
//!   181 KiB until the dead `lines[256]` array was removed from the ABI, 9.1
//!   KiB now.
//! - `suisei_engine_editor_band` memsets 107.5 KiB per call, and the face calls
//!   it **once per pane per paint** (`EditorHost.rows`).
//!
//! If both are flat here, the scaling the user feels is entirely on the Swift
//! side (SwiftUI body re-evaluation, `String` decode, `NSColor` conversion) and
//! no amount of Rust work will move it. That is the point of the test.
//!
//! ```text
//! cargo test -p suisei-engine --release --test scale_by_tabs_and_panes -- --ignored --nocapture
//! ```

use std::time::Instant;

use suisei_engine::ffi::{
    SuiseiBandC, SuiseiChromeSnapshot, SuiseiEngine, suisei_engine_chrome,
    suisei_engine_editor_band, suisei_engine_free, suisei_engine_new, suisei_engine_open_blank_tab,
    suisei_engine_split_horizontal, suisei_engine_split_vertical, suisei_engine_tick,
};

/// The face hands these in uninitialised (Swift zero-fills; the FFI memsets
/// again on entry). `zeroed` is the honest stand-in and keeps the 181 KiB off
/// the stack.
fn zeroed<T>() -> Box<T> {
    unsafe { Box::new(std::mem::MaybeUninit::<T>::zeroed().assume_init()) }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// A fresh engine with `tabs` blank buffers and `panes` split panes.
///
/// Blank tabs keep the document identical across every row, so the only thing
/// that varies is the count — which is exactly the variable under test.
fn engine_with(tabs: usize, panes: usize) -> *mut SuiseiEngine {
    let e = suisei_engine_new();
    for _ in 1..tabs {
        suisei_engine_open_blank_tab(e);
    }
    // Alternate the axis so three panes form a real tree rather than a row —
    // that is the shape `build_editor_surfaces` has to walk.
    for i in 1..panes {
        if i % 2 == 1 {
            suisei_engine_split_vertical(e);
        } else {
            suisei_engine_split_horizontal(e);
        }
    }
    e
}

/// Mean cost of one `suisei_engine_chrome` — the call the face makes on every
/// refresh, light path and full path alike.
fn chrome_ms(e: *mut SuiseiEngine, iters: usize) -> f64 {
    // Boxed: 181 KiB is far past a sensible stack frame, and the face's Swift
    // `SuiseiChromeSnapshot()` heap-allocates it the same way.
    let mut snap = zeroed::<SuiseiChromeSnapshot>();
    let t = Instant::now();
    for _ in 0..iters {
        let ok = suisei_engine_chrome(e, &mut *snap);
        std::hint::black_box(ok);
    }
    ms(t) / iters as f64
}

/// Mean cost of filling every pane's paint band once — one screen's worth of
/// rows for each pane, which is what one repaint costs at the boundary.
fn band_ms(e: *mut SuiseiEngine, panes: usize, iters: usize) -> f64 {
    let mut band = zeroed::<SuiseiBandC>();
    let t = Instant::now();
    for _ in 0..iters {
        for pane in 0..panes {
            let ok = suisei_engine_editor_band(e, pane as u32, 0, 60, 0, &mut *band);
            std::hint::black_box(ok);
        }
    }
    ms(t) / iters as f64
}

#[test]
#[ignore = "measurement, not an assertion"]
fn chrome_cost_by_tab_count() {
    println!();
    println!("=== suisei_engine_chrome, by open tab count ===");
    println!(
        "{:>6} {:>14} {:>26}",
        "tabs", "chrome pull", "per added tab"
    );
    let mut base = 0.0;
    for (i, tabs) in [1usize, 8, 16, 32, 64].into_iter().enumerate() {
        let e = engine_with(tabs, 1);
        let each = chrome_ms(e, 2_000);
        if i == 0 {
            base = each;
        }
        let slope = (each - base) / (tabs.max(1) as f64);
        println!("{tabs:>6} {each:>11.4}ms {slope:>23.5}ms");
        suisei_engine_free(e);
    }
    println!();
    println!("Flat here means tab-count lag is NOT in the engine. Baseline for");
    println!("this row, before `lines[256]` left the ABI: 0.0007ms at 1 tab,");
    println!("0.0010ms at 64 — the 181 KiB memset swamping everything else.");
}

#[test]
#[ignore = "measurement, not an assertion"]
fn paint_cost_by_pane_count() {
    println!();
    println!("=== one repaint's band pulls, by pane count ===");
    println!(
        "{:>6} {:>14} {:>14} {:>16}",
        "panes", "all bands", "per pane", "chrome pull"
    );
    for panes in [1usize, 2, 3, 4] {
        let e = engine_with(1, panes);
        let bands = band_ms(e, panes, 500);
        let chrome = chrome_ms(e, 2_000);
        println!(
            "{panes:>6} {bands:>11.4}ms {:>11.4}ms {chrome:>13.4}ms",
            bands / panes as f64
        );
        suisei_engine_free(e);
    }
    println!();
    println!("`all bands` is what ONE frame costs at the FFI boundary when every");
    println!("pane repaints. Linear in pane count is expected — each pane memsets");
    println!("its own 107.5 KiB SuiseiBandC (plus the same again on the Swift side).");
}

#[test]
#[ignore = "measurement, not an assertion"]
fn tick_cost_by_tabs_and_panes() {
    println!();
    println!("=== idle tick, tabs × panes ===");
    print!("{:>6}", "tabs\\panes");
    for panes in [1usize, 2, 3, 4] {
        print!("{panes:>12}");
    }
    println!();
    for tabs in [1usize, 8, 32, 64] {
        print!("{tabs:>6}");
        for panes in [1usize, 2, 3, 4] {
            let e = engine_with(tabs, panes);
            for _ in 0..8 {
                suisei_engine_tick(e, 50);
            }
            let t = Instant::now();
            for _ in 0..200 {
                std::hint::black_box(suisei_engine_tick(e, 50));
            }
            print!("{:>10.4}ms", ms(t) / 200.0);
            suisei_engine_free(e);
        }
        println!();
    }
    println!();
    println!("The tick is the engine's whole per-frame budget. If this stays");
    println!("under a microsecond across the grid, every millisecond the user");
    println!("feels is on the face side of the C ABI.");
}
