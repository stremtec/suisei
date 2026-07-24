//! Daemon entry point. Binds the Unix socket and serves clients until killed.
//! Started by a launchd LaunchAgent in production (arch plan §1.5); runnable by
//! hand for development: `cargo run -p suisei-daemon`.

use suisei_daemon::{protocol, server, state::DaemonState};

fn main() {
    let path = protocol::socket_path();
    match server::bind(&path) {
        Ok(listener) => {
            eprintln!(
                "suisei-daemon v{} listening on {}",
                protocol::PROTOCOL_VERSION,
                path.display()
            );
            server::serve(listener, DaemonState::new());
        }
        Err(e) => {
            eprintln!("suisei-daemon: cannot bind {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}
