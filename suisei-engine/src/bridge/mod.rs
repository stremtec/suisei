//! Bridge: normalize face events, schedule tick, hand off FrameDiff.
//! Business logic stays in Core/Compositor — this module stays thin.

pub mod input;

pub use input::{FfiKeyCode, key_from_ffi};
