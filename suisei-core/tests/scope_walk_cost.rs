//! Where the completion walk's nine milliseconds go.
//!
//! The app's own log reports the walk as one number: `rust scope walk`, 8.7 ms
//! at 50k lines and 3,194 top-level symbols, and 8.71/8.80 mean/max over four
//! samples — a spread of 0.09 ms. A cost that steady across different carets is
//! not responding to its input; it is doing the same work from scratch every
//! time. This splits it so the fix targets the part that actually costs.
//!
//! Ignored by default: it wants `perf-fixtures/big_50k.rs`, which is generated
//! and untracked.
//!
//!     cargo test -p suisei-core --test scope_walk_cost -- --ignored --nocapture

use std::time::Instant;

use suisei_core::scope::{self, GlobalScopeCache, ScopeLang};
use suisei_core::syntax::SyntaxEngine;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../perf-fixtures/big_50k.rs");

#[test]
#[ignore = "needs the generated 50k fixture; run explicitly"]
fn where_the_scope_walk_spends_its_time() {
    let Ok(src) = std::fs::read_to_string(FIXTURE) else {
        eprintln!("no fixture at {FIXTURE} — generate it first");
        return;
    };
    let lines = src.lines().count();

    let mut syntax = SyntaxEngine::new();
    let t = Instant::now();
    syntax.parse(&src, Some("rs"));
    let parse_ms = t.elapsed().as_secs_f64() * 1000.0;

    let Some((tree, text)) = syntax.live_tree() else {
        panic!("the fixture did not parse — the grammar is missing, not slow");
    };
    let lang = ScopeLang::from_ext("rs").expect("rust has a scope language");

    // A caret deep inside a function body, which is where completion actually
    // fires. Landing it on the last `individual_item` in the file puts a real
    // chain above it: block, function, impl, module, global.
    let byte: usize = text
        .rfind("individual_item")
        .map(|b| b + "individual_item".len())
        .expect("fixture shape changed");

    // Warm, then sample. The first call pays for page faults on a 2 MB string.
    let _ = scope::visible_at(tree, text, byte, lang);

    let mut samples = Vec::new();
    for i in 0..8 {
        // Vary the caret so a hidden per-caret cache cannot flatter the result.
        let b = text[..byte - i * 97].rfind("individual_item").unwrap_or(byte);
        let t = Instant::now();
        let syms = scope::visible_at(tree, text, b, lang);
        samples.push((t.elapsed().as_secs_f64() * 1000.0, syms.len()));
    }
    let total: f64 = samples.iter().map(|s| s.0).sum();
    let worst = samples.iter().map(|s| s.0).fold(0.0, f64::max);
    let count = samples[0].1;

    // The same call with the caret at the very top of the file. The scope
    // CHAIN there is one deep — no block, no function, no impl — while the
    // global scope collected at the end is identical. Whatever these two have
    // in common is the global collection; the difference is the chain.
    let mut shallow = Vec::new();
    for _ in 0..8 {
        let t = Instant::now();
        let syms = scope::visible_at(tree, text, 0, lang);
        shallow.push((t.elapsed().as_secs_f64() * 1000.0, syms.len()));
    }
    let shallow_mean: f64 = shallow.iter().map(|s| s.0).sum::<f64>() / shallow.len() as f64;

    println!("\n  fixture           {lines} lines, {} KiB", src.len() / 1024);
    println!("  parse             {parse_ms:.2} ms (once, off the typing path)");
    println!(
        "  visible_at        mean {:.2} ms, worst {worst:.2} ms, over {} samples",
        total / samples.len() as f64,
        samples.len()
    );
    println!("  symbols returned  {count}");
    println!(
        "  caret at byte 0   mean {shallow_mean:.2} ms, {} symbols  (chain is one deep)",
        shallow[0].1
    );
    println!(
        "  → chain build     {:.2} ms   → global collect  {shallow_mean:.2} ms",
        (total / samples.len() as f64) - shallow_mean
    );

    // The same walk with the global scope held across calls, which is what the
    // typing path does now. `tree_gen` is constant here because the tree is:
    // in the app it advances only when the worker delivers a new parse.
    let mut cache = GlobalScopeCache::default();
    let _ = scope::visible_at_cached(tree, text, byte, lang, &mut cache, 1, "");
    let mut cached = Vec::new();
    for i in 0..8 {
        let b = text[..byte - i * 97].rfind("individual_item").unwrap_or(byte);
        let t = Instant::now();
        let syms = scope::visible_at_cached(tree, text, b, lang, &mut cache, 1, "");
        cached.push((t.elapsed().as_secs_f64() * 1000.0, syms.len()));
    }
    let cached_mean: f64 = cached.iter().map(|s| s.0).sum::<f64>() / cached.len() as f64;
    let cached_worst = cached.iter().map(|s| s.0).fold(0.0, f64::max);
    println!(
        "  visible_at_cached mean {cached_mean:.2} ms, worst {cached_worst:.2} ms, {} symbols",
        cached[0].1
    );
    println!(
        "  → {:.1}x faster, and the first call after a reparse still pays the {shallow_mean:.2} ms\n",
        (total / samples.len() as f64) / cached_mean.max(0.0001)
    );

    // The cache must not change the ANSWER, only its cost.
    let plain = scope::visible_at(tree, text, byte, lang);
    let mut c2 = GlobalScopeCache::default();
    let via_cache = scope::visible_at_cached(tree, text, byte, lang, &mut c2, 1, "");
    assert_eq!(
        plain.iter().map(|s| &s.name).collect::<Vec<_>>(),
        via_cache.iter().map(|s| &s.name).collect::<Vec<_>>(),
        "the cached walk returned a different symbol list"
    );
    // …and a new tree generation must be noticed rather than served stale.
    let refreshed = scope::visible_at_cached(tree, text, byte, lang, &mut c2, 2, "");
    assert_eq!(refreshed.len(), plain.len(), "a new generation was not recollected");

    // Not an assertion on the number — this is a measurement, and a threshold
    // here would just be a flaky test on someone else's machine. It fails only
    // if the walk returns nothing, which would mean it is measuring an error
    // path rather than the work.
    assert!(count > 100, "expected the global scope, got {count} symbols");
}
