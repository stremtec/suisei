//! Daemon entry point. Binds the Unix socket and serves clients until killed.
//! Started by a launchd LaunchAgent in production (arch plan §1.5); runnable by
//! hand for development: `cargo run -p suisei-daemon`.

use suisei_daemon::{agent, protocol, server, state::DaemonState};

fn main() {
    let path = protocol::socket_path();
    // Is someone already doing this job?
    //
    // Starting a second daemon used to mean TAKING the socket — `bind` unlinked
    // it first — and the one that lost it kept running with a listener nothing
    // could reach. Every app launch added one. This is the other half of the
    // fix in `server::bind`: ask first, and act on the answer.
    match server::probe(&path) {
        Some(v) if v == protocol::PROTOCOL_VERSION => {
            eprintln!("suisei-daemon: v{v} already serving {} — nothing to do", path.display());
            return;
        }
        Some(v) => {
            // A daemon from an older app, left behind by an update. It cannot
            // serve this editor — the handshake would Nak — so it has to go,
            // and asking is how, rather than pulling the socket out from under
            // it and leaving it running.
            eprintln!("suisei-daemon: replacing v{v} (this is v{})", protocol::PROTOCOL_VERSION);
            if !server::request_shutdown(&path, std::time::Duration::from_secs(3)) {
                eprintln!("suisei-daemon: v{v} would not stand down; leaving it alone");
                std::process::exit(1);
            }
        }
        None => {}
    }
    match server::bind(&path) {
        Ok(listener) => {
            eprintln!(
                "suisei-daemon v{} listening on {}",
                protocol::PROTOCOL_VERSION,
                path.display()
            );
            // The daemon owns the menu-bar agent's presence — launch and keep it.
            agent::supervise_agent();
            server::serve(listener, DaemonState::new());
        }
        Err(e) => {
            eprintln!("suisei-daemon: cannot bind {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}
