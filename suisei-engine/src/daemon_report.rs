//! Tell the daemon what this editor is actually doing.
//!
//! The daemon does not own the language servers yet (that is the D1 migration
//! in `SUISEI-CURRENT-STATE.md`), so it can observe nothing about them on its
//! own — its `DaemonState` shipped with setters that had no production caller,
//! and the menu-bar agent has therefore always drawn `LSP none · DAP none ·
//! Project none`. This module is the missing writer: the engine tick offers a
//! [`Status`] built from live `App` state, and a background thread pushes it
//! over the same Unix socket the agent polls.
//!
//! Two rules shape it:
//!
//! * **The tick never blocks on the socket.** The channel is bounded at one
//!   slot and offered with `try_send`; a wedged or absent daemon costs the
//!   editor nothing, and a dropped update is re-sent by the next heartbeat.
//! * **Silence must decay.** The daemon expires reported fields after
//!   [`REPORT_TTL`](suisei_daemon::state::REPORT_TTL), so the heartbeat below
//!   has to stay comfortably inside it — otherwise a *live* editor would blink
//!   to "none" between updates.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use suisei_daemon::protocol::{Frame, Opcode, PROTOCOL_VERSION, Status, socket_path};

/// Re-send an unchanged status this often, so the daemon's TTL never expires a
/// live editor. Must stay well under `suisei_daemon::state::REPORT_TTL`.
const HEARTBEAT: Duration = Duration::from_secs(4);

/// After a failed connect, wait this long before trying again — a daemon that
/// is not running must not cost a `connect(2)` per report.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Bound on the handshake read, so a daemon that accepts and then goes quiet
/// cannot park the reporter thread forever.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// Decides *whether* a freshly built status is worth sending. Pure and
/// clock-injected so the policy is testable without a socket or a thread.
pub struct ReportGate {
    last: Option<Status>,
    last_sent_at: Option<Instant>,
}

impl ReportGate {
    pub fn new() -> Self {
        ReportGate {
            last: None,
            last_sent_at: None,
        }
    }

    /// True when `next` differs from the last sent status, or when the
    /// heartbeat is due. Records the decision, so call it once per offer.
    pub fn should_send(&mut self, next: &Status, now: Instant) -> bool {
        let changed = self.last.as_ref() != Some(next);
        let stale = match self.last_sent_at {
            Some(at) => now.duration_since(at) >= HEARTBEAT,
            None => true,
        };
        if changed || stale {
            self.last = Some(next.clone());
            self.last_sent_at = Some(now);
            return true;
        }
        false
    }
}

impl Default for ReportGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns the reporting thread. Dropping it closes the channel, which ends the
/// thread and the socket; the daemon's TTL then expires the last report — so a
/// closed editor stops claiming a live language server on its own.
pub struct Reporter {
    tx: SyncSender<Status>,
    gate: ReportGate,
}

impl Reporter {
    /// Spawn the reporting thread. Cheap and non-blocking: the socket is not
    /// touched until the first status arrives, and a missing daemon just means
    /// every send quietly fails.
    pub fn spawn() -> Reporter {
        // One slot: the newest status is the only one worth having.
        let (tx, rx) = sync_channel::<Status>(1);
        std::thread::Builder::new()
            .name("suisei-daemon-report".to_string())
            .spawn(move || run(rx))
            .ok();
        Reporter {
            tx,
            gate: ReportGate::new(),
        }
    }

    /// Offer the current status. Returns true when it was handed to the thread.
    /// Never blocks: a full channel means the previous update is still in
    /// flight, and dropping this one is correct — the next heartbeat re-sends.
    pub fn offer(&mut self, status: Status) -> bool {
        if !self.gate.should_send(&status, Instant::now()) {
            return false;
        }
        match self.tx.try_send(status) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

/// The reporting thread: keep one connection, re-establishing it lazily.
fn run(rx: Receiver<Status>) {
    let mut conn: Option<UnixStream> = None;
    let mut next_connect_attempt = Instant::now();

    while let Ok(status) = rx.recv() {
        // Coalesce anything that piled up while we were connecting.
        let status = drain_latest(&rx, status);

        if conn.is_none() {
            if Instant::now() < next_connect_attempt {
                continue;
            }
            conn = connect();
            if conn.is_none() {
                next_connect_attempt = Instant::now() + RECONNECT_BACKOFF;
                continue;
            }
        }
        if let Some(stream) = conn.as_mut() {
            if status.to_report_frame().write_to(stream).is_err() {
                // Daemon restarted or died — reconnect on the next status.
                conn = None;
                next_connect_attempt = Instant::now() + RECONNECT_BACKOFF;
            }
        }
    }
}

/// Take the newest status available without blocking.
fn drain_latest(rx: &Receiver<Status>, mut newest: Status) -> Status {
    loop {
        match rx.try_recv() {
            Ok(s) => newest = s,
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return newest,
        }
    }
}

/// Connect and complete the version handshake. `None` on any failure — a daemon
/// that is not running is the normal case, not an error worth logging per try.
fn connect() -> Option<UnixStream> {
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;
    Frame::control(Opcode::Hello).write_to(&mut stream).ok()?;
    let ack = Frame::read_from(&mut stream).ok()?;
    if ack.opcode != Opcode::HelloAck {
        // A `HelloNak` carries the daemon's version: an old daemon left running
        // from a previous build. Reporting into it would be silently wrong, so
        // say so once per connect attempt rather than pretending it worked.
        if ack.opcode == Opcode::HelloNak && ack.payload.len() >= 2 {
            let theirs = u16::from_le_bytes([ack.payload[0], ack.payload[1]]);
            eprintln!(
                "suisei: daemon speaks protocol v{theirs}, editor speaks v{PROTOCOL_VERSION} \
                 — status reporting disabled until the daemon is restarted"
            );
        }
        return None;
    }
    // Past the handshake nothing is read back (reports draw no reply); drop the
    // read timeout so a future request/response use is not surprised by it.
    let _ = stream.set_read_timeout(None);
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(project: &str) -> Status {
        Status {
            project: project.to_string(),
            ..Status::default()
        }
    }

    #[test]
    fn the_first_status_always_sends() {
        let mut gate = ReportGate::new();
        assert!(gate.should_send(&status("/a"), Instant::now()));
    }

    #[test]
    fn an_unchanged_status_is_skipped_until_the_heartbeat() {
        let mut gate = ReportGate::new();
        let t0 = Instant::now();
        assert!(gate.should_send(&status("/a"), t0));
        assert!(
            !gate.should_send(&status("/a"), t0 + Duration::from_secs(1)),
            "a tick that changed nothing must not touch the socket"
        );
        assert!(
            gate.should_send(&status("/a"), t0 + HEARTBEAT),
            "the heartbeat must fire or the daemon's TTL expires a live editor"
        );
    }

    #[test]
    fn a_changed_status_sends_immediately() {
        let mut gate = ReportGate::new();
        let t0 = Instant::now();
        assert!(gate.should_send(&status("/a"), t0));
        assert!(gate.should_send(&status("/b"), t0 + Duration::from_millis(50)));
    }

    /// The whole point of the heartbeat is to stay inside the daemon's window.
    #[test]
    fn the_heartbeat_is_well_inside_the_daemons_ttl() {
        assert!(
            HEARTBEAT * 2 < suisei_daemon::state::REPORT_TTL,
            "the daemon must tolerate a missed heartbeat without blinking to none"
        );
    }

    /// A reporter with no daemon listening must be inert, not fatal.
    #[test]
    fn offering_with_no_daemon_running_does_not_panic() {
        let mut r = Reporter::spawn();
        assert!(r.offer(status("/tmp/suisei-no-daemon")));
        assert!(
            !r.offer(status("/tmp/suisei-no-daemon")),
            "unchanged is skipped"
        );
    }
}
