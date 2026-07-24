//! suisei-daemon — the durable, headless process that owns external tool
//! lifecycles (rust-analyzer, debug adapters) and, in later phases, the
//! authoritative editor `App` state (SUISEI-ARCHITECTURE-PLAN.md §1).
//!
//! Phase order: this crate starts as the IPC substrate (`protocol` + `server`).
//! The LSP manager, then the DAP manager, plug into `server::handle_frame`; the
//! GUI's engine re-routes its LSP/DAP FFI through the socket. Owning the child
//! processes here is what makes them leak-proof and warm across GUI restarts.

pub mod protocol;
pub mod server;
