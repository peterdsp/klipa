//! Versioned request/response protocol between the klipa app (client) and
//! the privileged `klipa-helper` daemon (server).
//!
//! This crate is deliberately small, pure, and platform-agnostic: it is the
//! single source of truth for the wire format, so the app and the daemon
//! cannot drift apart, and the framing rules are unit-tested on every
//! platform. All OS work (sockets, `pmset`, code-signature checks) lives in
//! the two binaries, never here.
//!
//! # Shape
//!
//! The daemon owns the lid-closed override (`disablesleep`). The client
//! does not send shell commands, paths, settings keys, or environment:
//! every request is one of a closed set of structured operations
//! ([`Request`]), each carrying only typed, bounded fields. The daemon
//! answers with a structured [`Response`] that always reports its version,
//! the owning session generation, and the *effective* system state it
//! actually observed, so the client never has to infer protection from the
//! mere fact that it asked for it.
//!
//! # Ownership and leases
//!
//! A session is identified by a client-chosen [`Generation`] that only ever
//! increases within a run. The daemon tracks exactly one owning generation
//! at a time, so a delayed reply from an old start/stop cannot disturb a
//! newer session (it is rejected as [`ErrorReason::StaleGeneration`]).
//!
//! The override is held under a *lease*: the client renews it periodically,
//! and if the client dies, freezes, or is force-quit, the lease lapses and
//! the daemon restores normal sleep on its own. A frozen UI therefore
//! cannot strand a global override, and the daemon, not the UI event loop,
//! owns timed expiry. The lease is a crash-safety bound, not a session
//! ceiling: a healthy indefinite session simply keeps renewing.

use serde::{Deserialize, Serialize};

/// The wire protocol version. Bumped on any breaking change to the types
/// below so an old app and a new daemon (or vice versa) detect the
/// mismatch in the [`Request::Hello`] handshake instead of misreading each
/// other.
pub const PROTOCOL_VERSION: u32 = 1;

/// Fixed socket path. `/var/run` is root-owned and cleared on boot; the
/// daemon (RunAtLoad) recreates the socket each boot. Shared here so the
/// app and the daemon cannot disagree on it.
pub const SOCKET_PATH: &str = "/var/run/dev.peterdsp.klipa.helper.sock";

/// Hard cap on a single framed message, in bytes. One request or response
/// is a few dozen bytes of JSON; anything larger is hostile or broken, so
/// both ends refuse to read or write past this. Bounds memory and stops a
/// client that never sends a newline from growing a buffer without limit.
pub const MAX_FRAME_BYTES: usize = 4096;

/// A session generation: a client-chosen counter that only increases
/// within one app run. The daemon uses it to ignore stale operations from
/// an older session and to tie an override to exactly one owner.
pub type Generation = u64;

/// A request from the app to the daemon. A closed set of typed operations,
/// never an arbitrary command: the daemon can only ever be asked to do one
/// of these, so a compromised client cannot smuggle in shell, paths, keys,
/// or environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Version handshake. The client states the protocol version it speaks;
    /// the daemon rejects a mismatch rather than guessing.
    Hello { protocol: u32 },
    /// Read-only: report capabilities and the current effective state.
    Status,
    /// Acquire (or re-confirm, idempotently for the same generation) the
    /// lid-closed override, owned by `generation`. The daemon records the
    /// prior value, sets the flag, verifies it, and will restore on its own
    /// at whichever comes first: the session deadline, or a lapsed lease.
    ///
    /// * `lease_secs` is the crash-safety heartbeat: the client renews
    ///   before it elapses, so a dead or frozen client releases the
    ///   override on its own. It is deliberately short and is NOT a session
    ///   ceiling.
    /// * `session_secs` is the remaining session time (`None` for an
    ///   indefinite session). The daemon owns this expiry independently of
    ///   the UI, so an open menu or modal cannot delay or strand it.
    Begin {
        generation: Generation,
        lease_secs: u64,
        session_secs: Option<u64>,
    },
    /// Extend the lease (and refresh the remaining session time) on the
    /// override owned by `generation`. Cheap and frequent; this is the
    /// heartbeat that keeps a healthy session alive.
    Renew {
        generation: Generation,
        lease_secs: u64,
        session_secs: Option<u64>,
    },
    /// Release the override owned by `generation` and restore the prior
    /// value. Idempotent: ending an already-ended session is not an error.
    End { generation: Generation },
}

/// The effective lid-closed override state the daemon actually observed,
/// kept tri-state-plus-ownership so the client never confuses "I asked" or
/// "unreadable" with "protected".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effective {
    /// `disablesleep` is off: ordinary sleep behavior.
    Enabled,
    /// `disablesleep` is on and owned by the reported generation (klipa).
    DisabledOwned,
    /// `disablesleep` is on but klipa does not own it (set by the user or
    /// another tool), so klipa must not clear it.
    DisabledUnowned,
    /// `pmset` could not be read: neither on nor off can be asserted.
    Unknown,
}

/// A structured reply. Always carries the daemon's identity and version so
/// the client can detect an incompatible or stale daemon, plus the owning
/// generation and the effective state it measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub helper_version: String,
    pub protocol: u32,
    /// The generation that currently owns the override, or `0` if none.
    pub generation: Generation,
    /// The effective state the daemon measured while handling the request.
    pub effective: Effective,
    /// Seconds left on the current lease, when a session is active.
    pub lease_remaining_secs: Option<u64>,
    /// `None` on success; a specific reason on failure.
    pub error: Option<ErrorReason>,
    /// Human-readable detail for diagnostics (never secrets).
    pub detail: String,
}

/// Why a request failed. Specific enough that the menu can name the cause
/// and offer the right recovery, never a single vague "blocked".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorReason {
    /// The caller failed code-signature / Team ID validation.
    Unauthenticated,
    /// The client speaks a protocol version the daemon does not support.
    ProtocolMismatch,
    /// The operation referenced a generation older than the current owner.
    StaleGeneration,
    /// Renew/End for a generation the daemon does not currently own.
    NotOwner,
    /// `pmset` ran but the system never reflected the change (for example a
    /// managed Mac's power policy silently overriding it).
    SystemRefused,
    /// `pmset` could not be run at all.
    SystemUnavailable,
    /// The root-owned recovery journal could not be written, so the daemon
    /// refused to disable sleep with no way to recover it.
    JournalUnwritable,
    /// The request was unparseable, oversized, or otherwise malformed.
    Malformed,
}

impl Response {
    /// A fresh error response that still reports the daemon's identity.
    pub fn error(reason: ErrorReason, detail: impl Into<String>) -> Self {
        Response {
            helper_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            generation: 0,
            effective: Effective::Unknown,
            lease_remaining_secs: None,
            error: Some(reason),
            detail: detail.into(),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Encode one message as a single newline-terminated JSON line. Returns an
/// error if the encoded form would exceed [`MAX_FRAME_BYTES`], so an
/// oversized message is never put on the wire.
pub fn encode_line<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let mut bytes = serde_json::to_vec(msg).map_err(|_| FrameError::Encode)?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode one message from a single line (without the trailing newline).
/// A line at or over the cap, or that does not parse, is a framing error,
/// never a silently-accepted partial.
pub fn decode_line<T: for<'de> Deserialize<'de>>(line: &[u8]) -> Result<T, FrameError> {
    if line.len() >= MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    serde_json::from_slice(line).map_err(|_| FrameError::Decode)
}

/// What can go wrong framing a message. Kept separate from the protocol's
/// own [`ErrorReason`] because this is about the bytes, not the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The message is at or beyond [`MAX_FRAME_BYTES`].
    TooLarge,
    /// The bytes could not be parsed as the expected message.
    Decode,
    /// The message could not be serialized.
    Encode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_through_the_line_codec() {
        let cases = [
            Request::Hello { protocol: 1 },
            Request::Status,
            Request::Begin {
                generation: 7,
                lease_secs: 90,
                session_secs: Some(300),
            },
            Request::Renew {
                generation: 7,
                lease_secs: 90,
                session_secs: None,
            },
            Request::End { generation: 7 },
        ];
        for req in cases {
            let line = encode_line(&req).expect("encodes");
            assert_eq!(*line.last().unwrap(), b'\n', "framed with a newline");
            let decoded: Request = decode_line(&line[..line.len() - 1]).expect("decodes");
            assert_eq!(decoded, req);
        }
    }

    #[test]
    fn responses_round_trip() {
        let resp = Response {
            helper_version: "0.6.0".into(),
            protocol: PROTOCOL_VERSION,
            generation: 3,
            effective: Effective::DisabledOwned,
            lease_remaining_secs: Some(42),
            error: None,
            detail: String::new(),
        };
        let line = encode_line(&resp).expect("encodes");
        let decoded: Response = decode_line(&line[..line.len() - 1]).expect("decodes");
        assert_eq!(decoded, resp);
        assert!(decoded.is_ok());
    }

    #[test]
    fn an_oversized_message_is_refused_not_truncated() {
        // A detail far past the cap must fail to encode rather than be sent
        // as a truncated, misparsed frame.
        let resp = Response::error(ErrorReason::Malformed, "x".repeat(MAX_FRAME_BYTES));
        assert_eq!(encode_line(&resp), Err(FrameError::TooLarge));
    }

    #[test]
    fn a_line_at_the_cap_is_rejected_on_decode() {
        let big = vec![b'a'; MAX_FRAME_BYTES];
        let got: Result<Request, _> = decode_line(&big);
        assert_eq!(got, Err(FrameError::TooLarge));
    }

    #[test]
    fn garbage_is_a_decode_error_not_a_panic() {
        let got: Result<Request, _> = decode_line(b"{not json");
        assert_eq!(got, Err(FrameError::Decode));
        // An unknown op is also a clean decode error, never a default.
        let got: Result<Request, _> = decode_line(br#"{"op":"rm_rf","path":"/"}"#);
        assert_eq!(got, Err(FrameError::Decode));
    }

    #[test]
    fn unknown_fields_do_not_smuggle_extra_behavior() {
        // Extra fields on a known op are ignored (serde default), but the
        // op itself is still exactly what it claims, nothing more.
        let line = br#"{"op":"end","generation":5,"shell":"rm -rf /"}"#;
        let decoded: Request = decode_line(line).expect("decodes");
        assert_eq!(decoded, Request::End { generation: 5 });
    }
}
