//! Unix-socket server: accept clients, run the version handshake, then serve
//! frames until the peer hangs up. LSP/DAP dispatch plugs into `handle_frame`
//! in later steps; today it answers the handshake and heartbeat so the pipe is
//! provable end to end.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;

use crate::protocol::{Frame, Opcode, Status, PROTOCOL_VERSION};
use crate::state::DaemonState;

/// Bind the daemon socket, clearing a stale socket file from a prior run first
/// (a leftover node refuses `bind` with `EADDRINUSE`). Creates the parent dir.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    UnixListener::bind(path)
}

/// Accept connections forever, one handler thread per client. Blocks. Every
/// handler shares the one [`DaemonState`], so a status query reflects the
/// managers' live view.
pub fn serve(listener: UnixListener, state: Arc<DaemonState>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, state) {
                        if e.kind() != io::ErrorKind::UnexpectedEof {
                            eprintln!("suisei-daemon: client error: {e}");
                        }
                    }
                });
            }
            Err(e) => eprintln!("suisei-daemon: accept error: {e}"),
        }
    }
}

/// One connection: require a compatible `Hello`, then loop over frames until
/// EOF. A version mismatch gets a `HelloNak` (carrying the daemon's version)
/// and the connection closes — never a silent mis-decode.
pub fn handle_client(mut stream: UnixStream, state: Arc<DaemonState>) -> io::Result<()> {
    let hello = Frame::read_from(&mut stream)?;
    if hello.opcode != Opcode::Hello || hello.version != PROTOCOL_VERSION {
        Frame::new(Opcode::HelloNak, PROTOCOL_VERSION.to_le_bytes().to_vec())
            .write_to(&mut stream)?;
        return Ok(());
    }
    Frame::control(Opcode::HelloAck).write_to(&mut stream)?;

    loop {
        let frame = match Frame::read_from(&mut stream) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if let Some(reply) = handle_frame(&frame, &state) {
            reply.write_to(&mut stream)?;
        }
    }
}

/// Map a request frame to an optional reply. LSP/DAP request opcodes plug in
/// here as the managers land; for now it answers the heartbeat and the status
/// poll, adopts the editor's status push, and ignores anything unrecognised.
fn handle_frame(frame: &Frame, state: &DaemonState) -> Option<Frame> {
    match frame.opcode {
        Opcode::Ping => Some(Frame::control(Opcode::Pong)),
        Opcode::StatusRequest => Some(state.status().to_frame()),
        Opcode::ReportStatus => {
            // Fire-and-forget: the editor must not block its tick on us. A
            // payload we cannot decode is dropped rather than half-applied.
            if let Some(reported) = Status::decode(&frame.payload) {
                state.apply_report(&reported);
            }
            None
        }
        Opcode::Shutdown => {
            eprintln!("suisei-daemon: shutdown requested");
            // Ends the daemon and, with it, the agent supervisor thread.
            std::process::exit(0);
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    // Short /tmp path: macOS caps `sun_path` at ~104 bytes, and the temp dir is
    // already long. Unique suffix so parallel tests don't collide.
    fn tmp_sock(tag: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/tmp/suisei-d-{}-{tag}.sock", std::process::id()))
    }

    fn accept_one(listener: UnixListener) -> thread::JoinHandle<()> {
        let state = DaemonState::new();
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_client(stream, state);
            }
        })
    }

    fn accept_one_with(
        listener: UnixListener,
        state: std::sync::Arc<DaemonState>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_client(stream, state);
            }
        })
    }

    #[test]
    fn handshake_then_ping_pong_over_a_real_socket() {
        let path = tmp_sock("hs");
        let listener = bind(&path).unwrap();
        let server = accept_one(listener);

        let mut client = UnixStream::connect(&path).unwrap();
        Frame::control(Opcode::Hello).write_to(&mut client).unwrap();
        assert_eq!(Frame::read_from(&mut client).unwrap().opcode, Opcode::HelloAck);

        Frame::control(Opcode::Ping).write_to(&mut client).unwrap();
        assert_eq!(Frame::read_from(&mut client).unwrap().opcode, Opcode::Pong);

        drop(client); // EOF → handler returns cleanly
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn version_mismatch_is_refused_with_nak() {
        let path = tmp_sock("nak");
        let listener = bind(&path).unwrap();
        let server = accept_one(listener);

        let mut client = UnixStream::connect(&path).unwrap();
        let mut bad = Frame::control(Opcode::Hello);
        bad.version = 0xBEEF; // not PROTOCOL_VERSION
        bad.write_to(&mut client).unwrap();

        let reply = Frame::read_from(&mut client).unwrap();
        assert_eq!(reply.opcode, Opcode::HelloNak);
        assert_eq!(
            u16::from_le_bytes([reply.payload[0], reply.payload[1]]),
            PROTOCOL_VERSION
        );
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn status_request_returns_the_daemon_snapshot() {
        use crate::protocol::Status;
        let path = tmp_sock("status");
        let listener = bind(&path).unwrap();
        let state = DaemonState::new();
        state.set_lsp(1, 3); // one session, ready
        state.set_project("/Users/asill/suisei");
        let server = accept_one_with(listener, std::sync::Arc::clone(&state));

        let mut client = UnixStream::connect(&path).unwrap();
        Frame::control(Opcode::Hello).write_to(&mut client).unwrap();
        assert_eq!(Frame::read_from(&mut client).unwrap().opcode, Opcode::HelloAck);

        Frame::control(Opcode::StatusRequest).write_to(&mut client).unwrap();
        let reply = Frame::read_from(&mut client).unwrap();
        assert_eq!(reply.opcode, Opcode::StatusReport);
        let snap = Status::decode(&reply.payload).unwrap();
        assert_eq!(snap.lsp_sessions, 1);
        assert_eq!(snap.lsp_state, 3);
        assert_eq!(snap.project, "/Users/asill/suisei");

        drop(client);
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// The editor's push is the only way any of these fields become real: the
    /// daemon does not own the language servers, so nothing else can fill them.
    #[test]
    fn a_reported_status_lands_in_the_next_snapshot() {
        let path = tmp_sock("report");
        let listener = bind(&path).unwrap();
        let state = DaemonState::new();
        let server = accept_one_with(listener, std::sync::Arc::clone(&state));

        let mut client = UnixStream::connect(&path).unwrap();
        Frame::control(Opcode::Hello).write_to(&mut client).unwrap();
        assert_eq!(Frame::read_from(&mut client).unwrap().opcode, Opcode::HelloAck);

        // Before: the placeholder zeros the menu bar renders as "none".
        Frame::control(Opcode::StatusRequest).write_to(&mut client).unwrap();
        let before = Status::decode(&Frame::read_from(&mut client).unwrap().payload).unwrap();
        assert_eq!(before.lsp_state, 0);
        assert!(before.project.is_empty());

        Status {
            lsp_sessions: 1,
            lsp_state: 2, // indexing
            dap_state: 1, // running
            project: "/Users/asill/suisei".to_string(),
            ..Status::default()
        }
        .to_report_frame()
        .write_to(&mut client)
        .unwrap();

        // A report draws no reply, so the next read must be the status we ask
        // for — if the daemon answered the push, this read would desync.
        Frame::control(Opcode::StatusRequest).write_to(&mut client).unwrap();
        let after = Frame::read_from(&mut client).unwrap();
        assert_eq!(after.opcode, Opcode::StatusReport);
        let snap = Status::decode(&after.payload).unwrap();
        assert_eq!(snap.lsp_sessions, 1);
        assert_eq!(snap.lsp_state, 2);
        assert_eq!(snap.dap_state, 1);
        assert_eq!(snap.project, "/Users/asill/suisei");

        drop(client);
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// A short/garbled report must be dropped whole, never half-applied over
    /// good state.
    #[test]
    fn a_truncated_report_is_ignored() {
        let path = tmp_sock("badreport");
        let listener = bind(&path).unwrap();
        let state = DaemonState::new();
        state.set_project("/Users/asill/suisei");
        let server = accept_one_with(listener, std::sync::Arc::clone(&state));

        let mut client = UnixStream::connect(&path).unwrap();
        Frame::control(Opcode::Hello).write_to(&mut client).unwrap();
        assert_eq!(Frame::read_from(&mut client).unwrap().opcode, Opcode::HelloAck);

        Frame::new(Opcode::ReportStatus, vec![0u8; 8]).write_to(&mut client).unwrap();

        Frame::control(Opcode::StatusRequest).write_to(&mut client).unwrap();
        let snap = Status::decode(&Frame::read_from(&mut client).unwrap().payload).unwrap();
        assert_eq!(snap.project, "/Users/asill/suisei", "state must survive garbage");

        drop(client);
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_clears_a_stale_socket_file() {
        let path = tmp_sock("stale");
        let l1 = bind(&path).unwrap();
        drop(l1); // leaves the socket node on disk
        // Second bind must succeed by removing the stale node.
        let _l2 = bind(&path).unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
