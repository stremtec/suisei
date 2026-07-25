//! Where the 20 Hz tick actually spends its time on a large file.
//!
//! `keystroke_latency` showed a single ~57 ms spike per run on a 20,000-line
//! document, identical whether the buffer is dirty or clean. This splits the
//! tick into its parts so the spike gets a name instead of a guess.
//!
//! ```text
//! cargo test -p suisei-engine --release --test tick_breakdown -- --ignored --nocapture
//! ```

use std::time::Instant;

use suisei_engine::Engine;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn engine_with(lines: usize) -> Engine {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let mut text = String::new();
    for i in 0..lines {
        if i % 20 == 0 {
            text.push_str(&format!("fn function_number_{i}(argument: &str) -> usize {{\n"));
        } else if i % 20 == 19 {
            text.push_str("}\n");
        } else {
            text.push_str("    let value = some_call(argument, other_argument);\n");
        }
    }
    engine.app.buffer = suisei_core::buffer::Buffer::from_string(&text);
    engine.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_breakdown.rs"));
    engine
}

#[test]
#[ignore = "measurement, not an assertion"]
fn tick_breakdown_by_file_size() {
    println!();
    println!("=== one tick, by part (release build) ===");
    println!(
        "{:<8} {:>13} {:>13} {:>13} {:>13}",
        "lines", "buffer.text", "build_outline", "idle tick", "post-edit max"
    );
    for lines in [2_000usize, 20_000, 60_000] {
        let mut engine = engine_with(lines);
        for _ in 0..16 {
            engine.tick(50); // warm every cache
        }

        // What the shadow-WAL call site builds unconditionally, every tick.
        let t = Instant::now();
        for _ in 0..20 {
            std::hint::black_box(engine.app.buffer.text());
        }
        let text_ms = ms(t) / 20.0;

        let t = Instant::now();
        for _ in 0..5 {
            std::hint::black_box(suisei_engine::compositor::build_outline_public(&engine.app));
        }
        let outline_ms = ms(t) / 5.0;

        // A tick with nothing to do at all.
        let t = Instant::now();
        for _ in 0..20 {
            engine.tick(50);
        }
        let idle_ms = ms(t) / 20.0;

        // The real pattern: edit, then let the idle-refresh window elapse. One
        // of the next twelve ticks pays the outline rebuild + full compose.
        let mut worst: f64 = 0.0;
        for _ in 0..4 {
            engine.app.buffer.touch();
            for _ in 0..13 {
                let t = Instant::now();
                engine.tick(50);
                worst = worst.max(ms(t));
            }
        }

        println!(
            "{lines:<8} {text_ms:>12.3}ms {outline_ms:>12.3}ms {idle_ms:>12.3}ms {worst:>12.3}ms"
        );
    }
    println!();
    println!("`buffer.text` is what the shadow-WAL call site builds EVERY tick,");
    println!("dirty or not. `post-edit max` is the hitch a user feels ~600ms after");
    println!("they stop typing: the idle outline refresh forcing a full compose.");
    println!();
}
