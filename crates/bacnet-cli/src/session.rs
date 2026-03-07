//! Session state for the interactive shell.
//!
//! Tracks default target, BBMD registration, and other per-session state.

use owo_colors::OwoColorize;
use tokio::task::JoinHandle;

/// Per-session state shared across shell commands.
pub struct Session {
    /// Default target address (MAC bytes) set via `target` command.
    pub default_target: Option<Vec<u8>>,
    /// Human-readable default target string for display.
    pub default_target_display: Option<String>,
    /// Active BBMD registration info.
    pub bbmd_registration: Option<BbmdRegistration>,
    /// Background task handle for BBMD auto-renewal.
    bbmd_renewal_task: Option<JoinHandle<()>>,
}

/// Active BBMD foreign device registration.
pub struct BbmdRegistration {
    /// BBMD address (MAC bytes) for future use (e.g. unregister).
    #[allow(dead_code)]
    pub bbmd_mac: Vec<u8>,
    /// Human-readable BBMD address.
    pub bbmd_display: String,
    /// TTL in seconds.
    pub ttl: u16,
}

impl Session {
    pub fn new() -> Self {
        Self {
            default_target: None,
            default_target_display: None,
            bbmd_registration: None,
            bbmd_renewal_task: None,
        }
    }

    /// Set the default target.
    pub fn set_target(&mut self, mac: Vec<u8>, display: String) {
        self.default_target = Some(mac);
        self.default_target_display = Some(display);
    }

    /// Clear the default target.
    pub fn clear_target(&mut self) {
        self.default_target = None;
        self.default_target_display = None;
    }

    /// Register BBMD and start auto-renewal background task.
    /// `renew_fn` is called at 80% of TTL to re-register.
    pub fn set_bbmd_registration(
        &mut self,
        bbmd_mac: Vec<u8>,
        bbmd_display: String,
        ttl: u16,
        renew_fn: impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
            + Send
            + 'static,
    ) {
        // Cancel any existing renewal task
        self.cancel_bbmd_renewal();

        self.bbmd_registration = Some(BbmdRegistration {
            bbmd_mac,
            bbmd_display,
            ttl,
        });

        // Spawn renewal task at 80% of TTL
        let renewal_interval = std::time::Duration::from_secs((ttl as u64) * 80 / 100);
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(renewal_interval).await;
                match renew_fn().await {
                    Ok(()) => {
                        eprintln!("{}", "[BBMD registration renewed]".dimmed());
                    }
                    Err(e) => {
                        eprintln!("{}", format!("[BBMD renewal failed: {e}]").red());
                    }
                }
            }
        });
        self.bbmd_renewal_task = Some(handle);
    }

    /// Cancel BBMD registration and renewal task.
    pub fn cancel_bbmd_renewal(&mut self) {
        if let Some(handle) = self.bbmd_renewal_task.take() {
            handle.abort();
        }
        self.bbmd_registration = None;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel_bbmd_renewal();
    }
}
