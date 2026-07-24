//! Suisei engine: owns `App`, runs compose, exposes a C ABI for the Swift face.
//!
//! Layers (see docs/SUISEI-PLAN.md §3):
//! - Core: `suisei_core::App` + `dispatch`
//! - Compositor: `compositor` (Scene / FrameDiff stubs)
//! - Bridge logic: `bridge` + `ffi` (only face exit)
//! - Renderer: Swift app (not in this crate)

pub mod bridge;
pub mod compositor;
pub mod ffi;
pub mod journal;
pub mod runtime;

pub use runtime::Engine;
