//! Wire protocol for the GUI ↔ daemon Unix socket.
//!
//! One frame on the wire:
//!
//! ```text
//! ┌────────────┬───────────┬────────────┬──────────────────────┐
//! │ u32 len LE │ u16 op LE │ u16 ver LE │ payload (len-4 bytes) │
//! └────────────┴───────────┴────────────┴──────────────────────┘
//! ```
//!
//! `len` counts everything after itself (opcode + version + payload), so a
//! frame with an empty payload has `len == 4`. `ver` is **per-opcode**: the
//! payload layout for an opcode may evolve, and a receiver rejects a version it
//! does not understand rather than decoding the wrong fixed offsets silently
//! (SUISEI-ARCHITECTURE-PLAN.md §1.3). Payloads are raw fixed-layout bytes — no
//! serde, matching the FFI structs the GUI already decodes.

use std::io::{self, Read, Write};
use std::path::PathBuf;

/// Reject absurd frames instead of allocating gigabytes on a desync/garbage
/// byte. 64 MiB is far above any real editor payload (the largest FFI snapshot
/// is well under 1 MiB).
pub const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

/// Message kinds. Numeric values are wire-stable — append, never renumber.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum Opcode {
    /// Client → daemon: open a session. Payload carries the client's protocol
    /// version so a mismatch is caught at the handshake, not mid-stream.
    Hello = 1,
    /// Daemon → client: handshake accepted.
    HelloAck = 2,
    /// Daemon → client: handshake refused (version mismatch). Payload: the
    /// daemon's `PROTOCOL_VERSION` as `u16 LE`, so the client can report it.
    HelloNak = 3,
    /// Liveness probe / heartbeat.
    Ping = 4,
    Pong = 5,
    /// Client → daemon: request the current daemon/LSP/DAP status snapshot
    /// (empty payload). The menu-bar agent polls this.
    StatusRequest = 6,
    /// Daemon → client: a [`Status`] snapshot (see its layout).
    StatusReport = 7,
    /// Client → daemon: stop the daemon (and, with it, the supervised agent).
    /// The menu-bar "Quit" sends this so quitting actually ends the daemon
    /// rather than letting the supervisor respawn the agent.
    Shutdown = 8,
    /// Editor → daemon: "here is my live LSP/DAP/project state". Payload is the
    /// same [`Status`] layout the daemon reports back, and there is no reply.
    ///
    /// The daemon does not own the language servers yet (that is the D1
    /// migration), so it cannot observe any of this itself — without this
    /// opcode every field it reports is a placeholder zero. `uptime_secs` is
    /// ignored on the way in: uptime belongs to the daemon, not the reporter.
    ReportStatus = 9,
    /// Reserved: anything the receiver does not recognise decodes to this and
    /// is dropped (with a Nak for requests), never misinterpreted.
    Unknown = 0xFFFF,
}

impl Opcode {
    pub fn from_u16(v: u16) -> Self {
        match v {
            1 => Opcode::Hello,
            2 => Opcode::HelloAck,
            3 => Opcode::HelloNak,
            4 => Opcode::Ping,
            5 => Opcode::Pong,
            6 => Opcode::StatusRequest,
            7 => Opcode::StatusReport,
            8 => Opcode::Shutdown,
            9 => Opcode::ReportStatus,
            _ => Opcode::Unknown,
        }
    }
    pub fn as_u16(self) -> u16 {
        self as u16
    }
}

/// Bumped when the framing or the handshake contract changes. Individual
/// opcodes carry their own `version` for payload evolution.
pub const PROTOCOL_VERSION: u16 = 1;

/// A decoded frame. `payload` is opcode-specific fixed-layout bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    pub opcode: Opcode,
    pub version: u16,
    pub payload: Vec<u8>,
}

impl Frame {
    /// A frame at the current protocol version with the given payload.
    pub fn new(opcode: Opcode, payload: Vec<u8>) -> Self {
        Frame { opcode, version: PROTOCOL_VERSION, payload }
    }

    /// An empty-payload control frame (Hello/Ping/Pong/…).
    pub fn control(opcode: Opcode) -> Self {
        Frame::new(opcode, Vec::new())
    }

    /// Serialise to the wire layout.
    pub fn encode(&self) -> Vec<u8> {
        let body_len = 4 + self.payload.len(); // opcode(2) + version(2) + payload
        let mut out = Vec::with_capacity(4 + body_len);
        out.extend_from_slice(&(body_len as u32).to_le_bytes());
        out.extend_from_slice(&self.opcode.as_u16().to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Write the frame and flush it.
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.encode())?;
        w.flush()
    }

    /// Read exactly one frame. Returns `UnexpectedEof` at a clean stream end.
    pub fn read_from(r: &mut impl Read) -> io::Result<Frame> {
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf);
        if len < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame shorter than its header",
            ));
        }
        if len > MAX_FRAME_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame exceeds MAX_FRAME_LEN",
            ));
        }
        let mut body = vec![0u8; len as usize];
        r.read_exact(&mut body)?;
        let opcode = Opcode::from_u16(u16::from_le_bytes([body[0], body[1]]));
        let version = u16::from_le_bytes([body[2], body[3]]);
        Ok(Frame {
            opcode,
            version,
            payload: body[4..].to_vec(),
        })
    }
}

/// Fixed capacity of the project-path field in an encoded [`Status`].
pub const STATUS_PROJECT_CAP: usize = 512;
/// Total wire size of an encoded [`Status`] payload.
pub const STATUS_LEN: usize = 16 + STATUS_PROJECT_CAP;

/// The daemon's health snapshot, shown by the menu-bar agent. Fixed binary
/// layout (no serde), like every payload:
///
/// ```text
/// 0  u16 lsp_sessions   2 u8 lsp_state   3 u8 dap_state   4 u8 health
/// 8  u64 uptime_secs    16 [u8;512] project (NUL-terminated UTF-8)
/// ```
///
/// `lsp_state`: 0 none · 1 starting · 2 indexing · 3 ready · 4 error.
/// `dap_state`: 0 none · 1 running · 2 paused.
/// `health`:    0 starting · 1 healthy · 2 degraded.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Status {
    pub lsp_sessions: u16,
    pub lsp_state: u8,
    pub dap_state: u8,
    pub health: u8,
    pub uptime_secs: u64,
    /// Indexed project root; empty when none is loaded.
    pub project: String,
}

impl Status {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; STATUS_LEN];
        out[0..2].copy_from_slice(&self.lsp_sessions.to_le_bytes());
        out[2] = self.lsp_state;
        out[3] = self.dap_state;
        out[4] = self.health;
        // bytes 5..8 are padding
        out[8..16].copy_from_slice(&self.uptime_secs.to_le_bytes());
        let p = self.project.as_bytes();
        let n = p.len().min(STATUS_PROJECT_CAP - 1); // always leave a NUL
        out[16..16 + n].copy_from_slice(&p[..n]);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Status> {
        if payload.len() < STATUS_LEN {
            return None;
        }
        let uptime_secs = u64::from_le_bytes([
            payload[8], payload[9], payload[10], payload[11], payload[12], payload[13],
            payload[14], payload[15],
        ]);
        let raw = &payload[16..16 + STATUS_PROJECT_CAP];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Some(Status {
            lsp_sessions: u16::from_le_bytes([payload[0], payload[1]]),
            lsp_state: payload[2],
            dap_state: payload[3],
            health: payload[4],
            uptime_secs,
            project: String::from_utf8_lossy(&raw[..end]).into_owned(),
        })
    }

    /// Wrap this snapshot in a `StatusReport` frame.
    pub fn to_frame(&self) -> Frame {
        Frame::new(Opcode::StatusReport, self.encode())
    }

    /// Wrap this snapshot in a `ReportStatus` frame — the editor's push toward
    /// the daemon. Same bytes, opposite direction.
    pub fn to_report_frame(&self) -> Frame {
        Frame::new(Opcode::ReportStatus, self.encode())
    }
}

/// Where the daemon listens and clients connect. Prefers `$XDG_RUNTIME_DIR`
/// (per-user, tmpfs, auto-cleaned); falls back to the macOS app-support dir.
/// The parent directory is the caller's responsibility to create.
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("suisei").join("daemon.sock");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Suisei")
        .join("daemon.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips_through_bytes() {
        let f = Frame::new(Opcode::Hello, vec![1, 2, 3, 4, 5]);
        let bytes = f.encode();
        // len prefix counts opcode+version+payload = 4 + 5 = 9.
        assert_eq!(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), 9);
        let mut cursor = std::io::Cursor::new(bytes);
        let back = Frame::read_from(&mut cursor).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn control_frame_has_empty_payload_and_len_four() {
        let f = Frame::control(Opcode::Ping);
        let bytes = f.encode();
        assert_eq!(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), 4);
        let mut cursor = std::io::Cursor::new(bytes);
        let back = Frame::read_from(&mut cursor).unwrap();
        assert_eq!(back.opcode, Opcode::Ping);
        assert!(back.payload.is_empty());
    }

    #[test]
    fn unknown_opcode_decodes_to_unknown_not_a_panic() {
        let mut raw = Frame::new(Opcode::Ping, vec![9]).encode();
        // Overwrite the opcode bytes (offset 4,5) with a value we never assign.
        raw[4] = 0xAB;
        raw[5] = 0xCD;
        let mut cursor = std::io::Cursor::new(raw);
        let back = Frame::read_from(&mut cursor).unwrap();
        assert_eq!(back.opcode, Opcode::Unknown);
    }

    #[test]
    fn oversized_len_is_rejected_before_allocating() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&(MAX_FRAME_LEN + 1).to_le_bytes());
        let mut cursor = std::io::Cursor::new(raw);
        let err = Frame::read_from(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_body_is_an_error_not_a_hang() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&8u32.to_le_bytes()); // claims 8 body bytes
        raw.extend_from_slice(&[1, 0, 1, 0]); // only 4 provided
        let mut cursor = std::io::Cursor::new(raw);
        assert!(Frame::read_from(&mut cursor).is_err());
    }

    #[test]
    fn status_round_trips_including_project_path() {
        let s = Status {
            lsp_sessions: 2,
            lsp_state: 3,
            dap_state: 1,
            health: 1,
            uptime_secs: 4242,
            project: "/Users/asill/suisei".to_string(),
        };
        let bytes = s.encode();
        assert_eq!(bytes.len(), STATUS_LEN);
        assert_eq!(Status::decode(&bytes).unwrap(), s);
    }

    #[test]
    fn status_decode_rejects_short_payload() {
        assert!(Status::decode(&[0u8; 4]).is_none());
    }

    #[test]
    fn socket_path_prefers_xdg_runtime_dir() {
        // Non-destructive: only asserts the shape given a set var.
        let prev = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/tmp/xdg-test") };
        assert_eq!(socket_path(), PathBuf::from("/tmp/xdg-test/suisei/daemon.sock"));
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) },
            None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
        }
    }
}
