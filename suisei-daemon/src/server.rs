//! Unix-socket server: accept clients, run the version handshake, then serve
//! frames until the peer hangs up. LSP/DAP dispatch plugs into `handle_frame`
//! in later steps; today it answers the handshake and heartbeat so the pipe is
//! provable end to end.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;

use crate::protocol::{Frame, Opcode, PROTOCOL_VERSION};

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

/// Accept connections forever, one handler thread per client. Blocks.
pub fn serve(listener: UnixListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream) {
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
pub fn handle_client(mut stream: UnixStream) -> io::Result<()> {
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
        if let Some(reply) = handle_frame(&frame) {
            reply.write_to(&mut stream)?;
        }
    }
}

/// Map a request frame to an optional reply. LSP/DAP opcodes will be handled
/// here once the managers land; for now it answers the heartbeat and ignores
/// anything unrecognised (a stricter Nak comes with the request opcodes).
fn handle_frame(frame: &Frame) -> Option<Frame> {
    match frame.opcode {
        Opcode::Ping => Some(Frame::control(Opcode::Pong)),
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
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_client(stream);
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
    fn bind_clears_a_stale_socket_file() {
        let path = tmp_sock("stale");
        let l1 = bind(&path).unwrap();
        drop(l1); // leaves the socket node on disk
        // Second bind must succeed by removing the stale node.
        let _l2 = bind(&path).unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
