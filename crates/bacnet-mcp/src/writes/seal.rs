//! The write confirmation state (design §7.4): what a `bacnet_write` hands
//! the client between the preview and the retry, as MCP's `requestState`.
//!
//! The client may change anything it echoes, so the state is sealed with
//! XChaCha20-Poly1305 under a key drawn for this process: the principal and
//! the tool are the associated data (so another caller's state fails to
//! open), and the sealed body holds a digest of the arguments, a digest of
//! the preview, a nonce and an expiry 120 s out. Each nonce opens once.
//! The key never leaves the process, so no state outlives it; the expiry
//! runs on the process's own clock (Tokio's, which tests can pause).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

/// How long a confirmation stays good: the TUI's preview timeout.
pub(crate) const LIFETIME: Duration = Duration::from_secs(120);

/// What a confirmation binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct State {
    /// A digest of the call's arguments.
    pub(crate) args: String,
    /// A digest of the preview the person confirmed.
    pub(crate) preview: String,
    /// Minutes the confirmation arms a write session for, if it does.
    pub(crate) arm: Option<u32>,
    /// Single use: opened once at most.
    nonce: String,
    /// When it lapses, in milliseconds on the process's clock.
    expires_ms: u64,
}

/// Why a state was refused. Nothing is sent for any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsealed {
    /// Not one of ours, changed, or sealed for another caller or tool.
    Forged,
    /// Past its 120 s.
    Expired,
    /// Opened before.
    Replayed,
}

impl Unsealed {
    /// The refusal reason.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Forged => "confirmation-invalid",
            Self::Expired => "confirmation-expired",
            Self::Replayed => "confirmation-replayed",
        }
    }
}

/// Seals and opens one process's confirmation states.
pub(crate) struct Sealer {
    cipher: XChaCha20Poly1305,
    epoch: Instant,
    /// Nonces opened, with their expiry: one opened twice is a replay. An
    /// entry goes once its state would have expired anyway.
    used: Mutex<HashMap<String, u64>>,
}

impl Sealer {
    /// A sealer under a fresh random key.
    pub(crate) fn new() -> Self {
        let mut key = [0; 32];
        // The OS's random source; a process without one can't seal.
        getrandom::fill(&mut key).expect("the OS random source for the confirmation key");
        let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("a 32-octet key");
        Self {
            cipher,
            epoch: Instant::now(),
            used: Mutex::new(HashMap::new()),
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The associated data: the caller and the tool.
    fn aad(principal: &str, tool: &str) -> Vec<u8> {
        let mut aad = b"bacnet-mcp confirmation v1\0".to_vec();
        aad.extend_from_slice(principal.as_bytes());
        aad.push(0);
        aad.extend_from_slice(tool.as_bytes());
        aad
    }

    /// Seal a confirmation of `preview` (a digest) for these arguments,
    /// for `principal` calling `tool`.
    pub(crate) fn seal(
        &self,
        principal: &str,
        tool: &str,
        args: String,
        preview: String,
        arm: Option<u32>,
    ) -> String {
        let mut random = [0u8; 40];
        getrandom::fill(&mut random).expect("the OS random source for a nonce");
        let (aead_nonce, state_nonce) = random.split_at(24);
        let state = State {
            args,
            preview,
            arm,
            nonce: hex(state_nonce),
            expires_ms: self.now_ms().saturating_add(LIFETIME.as_millis() as u64),
        };
        let body = serde_json::to_vec(&state).expect("a state serializes");
        let nonce = XNonce::try_from(aead_nonce).expect("24 octets");
        let sealed = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &body,
                    aad: &Self::aad(principal, tool),
                },
            )
            .expect("sealing never fails for a short body");
        format!("{}.{}", hex(aead_nonce), hex(&sealed))
    }

    /// Open `token` for `principal` calling `tool`: once, before it
    /// expires, and only if this process sealed it for them.
    pub(crate) fn open(&self, token: &str, principal: &str, tool: &str) -> Result<State, Unsealed> {
        let (nonce, sealed) = token.split_once('.').ok_or(Unsealed::Forged)?;
        let nonce = unhex(nonce).ok_or(Unsealed::Forged)?;
        let sealed = unhex(sealed).ok_or(Unsealed::Forged)?;
        let nonce = XNonce::try_from(nonce.as_slice()).map_err(|_| Unsealed::Forged)?;
        let body = self
            .cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &sealed,
                    aad: &Self::aad(principal, tool),
                },
            )
            .map_err(|_| Unsealed::Forged)?;
        let state: State = serde_json::from_slice(&body).map_err(|_| Unsealed::Forged)?;
        let now = self.now_ms();
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        used.retain(|_, expires| *expires >= now);
        if used.contains_key(&state.nonce) {
            return Err(Unsealed::Replayed);
        }
        // Spent now, whatever happens next: a confirmation opens once.
        used.insert(state.nonce.clone(), state.expires_ms);
        if state.expires_ms < now {
            return Err(Unsealed::Expired);
        }
        Ok(state)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed(sealer: &Sealer) -> String {
        sealer.seal("alice", "bacnet_write", "a".into(), "p".into(), None)
    }

    #[tokio::test(start_paused = true)]
    async fn a_state_opens_once_for_its_caller_until_it_expires() {
        let sealer = Sealer::new();
        let token = sealed(&sealer);
        let state = sealer.open(&token, "alice", "bacnet_write").unwrap();
        assert_eq!((state.args.as_str(), state.preview.as_str()), ("a", "p"));
        assert_eq!(
            sealer.open(&token, "alice", "bacnet_write"),
            Err(Unsealed::Replayed)
        );
        // Another principal or tool can't open it, nor can another process.
        let token = sealed(&sealer);
        assert_eq!(
            sealer.open(&token, "mallory", "bacnet_write"),
            Err(Unsealed::Forged)
        );
        assert_eq!(
            sealer.open(&token, "alice", "bacnet_revert"),
            Err(Unsealed::Forged)
        );
        assert_eq!(
            Sealer::new().open(&token, "alice", "bacnet_write"),
            Err(Unsealed::Forged)
        );
        // One octet changed anywhere fails.
        let mut tampered = sealed(&sealer).into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert_eq!(
            sealer.open(&tampered, "alice", "bacnet_write"),
            Err(Unsealed::Forged)
        );
        // Past 120 s it has expired.
        let token = sealed(&sealer);
        tokio::time::advance(LIFETIME + Duration::from_millis(1)).await;
        assert_eq!(
            sealer.open(&token, "alice", "bacnet_write"),
            Err(Unsealed::Expired)
        );
    }

    /// The state is sealed, not just signed: the client sees no digest.
    #[test]
    fn the_body_is_not_readable() {
        let sealer = Sealer::new();
        let token = sealer.seal(
            "alice",
            "bacnet_write",
            "args-digest".into(),
            "p".into(),
            None,
        );
        let (_, body) = token.split_once('.').unwrap();
        let text = String::from_utf8_lossy(&unhex(body).unwrap()).to_string();
        assert!(!text.contains("args-digest"), "{text}");
    }
}
