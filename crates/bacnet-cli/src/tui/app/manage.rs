//! `:dcc` and `:reinit` in the app (#1799): the refusals, the masked
//! password prompt, and the hand-off to the write session's T3
//! confirmation.
//!
//! The target is the device the operator has in front of them, as for
//! `w`: the open device on the Browse screen, or the selected row of the
//! device table on the Devices screen. The password is typed into a
//! [`Secret`] and shown only as a mask; Esc drops it with the prompt, and
//! a declined confirmation drops it before anything is recorded.

use std::time::Instant;

use super::safety::{ARM_HINT, Modal, ModalKey, modal};
use super::watch::Screen;
use super::{App, FLASH_FOR};
use crate::tui::safety::Change;
use crate::tui::safety::audit::Secret;
use crate::tui::safety::manage::{Action, MAX_PASSWORD, NEEDS_DANGEROUS, Request};

/// What the operator is told when nothing is selected.
pub(crate) const NO_DEVICE: &str =
    "No device selected: pick one on the Devices screen (1) or open one in Browse (2).";

/// What a monitor-only session says.
pub(crate) const MONITOR_ONLY: &str =
    "Monitor-only: this session listens and transmits nothing; dcc and reinit are refused.";

impl App {
    /// The device `:dcc` and `:reinit` go to: the browser's on the Browse
    /// screen, the selected row's on the Devices screen, none elsewhere.
    fn manage_target(&mut self) -> Option<u32> {
        if let Some(browser) = self.shown_browser() {
            return Some(browser.device);
        }
        if self.screen != Screen::Main {
            return None;
        }
        self.settle_devices();
        self.devices.selected_row().map(|row| row.instance)
    }

    /// Why `:dcc` and `:reinit` may not run now, if they may not; the
    /// target is checked apart.
    fn manage_refusal(&self) -> Option<String> {
        if self.safety.passive {
            return Some(match self.monitor {
                Some(_) => MONITOR_ONLY.into(),
                None => "Passive: this session transmits nothing.".into(),
            });
        }
        let Some(policy) = &self.safety.policy else {
            return Some("Not connected: device management needs a connected session.".into());
        };
        if !policy.dangerous {
            return Some(format!("Refused: {NEEDS_DANGEROUS}."));
        }
        if self.safety.armed.is_none() {
            return Some(ARM_HINT.into());
        }
        None
    }

    /// `:dcc` or `:reinit`: refused with its reason, or the masked password
    /// prompt for the device in front of the operator.
    pub(super) fn open_manage(&mut self, action: Action) {
        if let Some(why) = self.manage_refusal() {
            self.flash(why, FLASH_FOR);
            return;
        }
        let Some(device) = self.manage_target() else {
            self.flash(NO_DEVICE, FLASH_FOR);
            return;
        };
        let request = Box::new(Request {
            device,
            action,
            password: None,
        });
        self.overlay = modal(Modal::Password {
            request,
            typed: Secret::default(),
            opened: self.now,
        });
    }

    /// A key at the password prompt: typed characters go into the secret,
    /// Enter moves on to the T3 confirmation, Esc cancels and drops what
    /// was typed.
    pub(super) fn password_key(
        &mut self,
        key: ModalKey,
        mut request: Box<Request>,
        mut typed: Secret,
        opened: Instant,
    ) -> Option<Modal> {
        match key {
            // What was typed goes with the prompt.
            ModalKey::Esc => {
                self.flash(
                    "Cancelled: nothing was sent, and the password was cleared.",
                    FLASH_FOR,
                );
                return None;
            }
            ModalKey::Char(ch) if typed.chars() < MAX_PASSWORD => typed.push(ch),
            ModalKey::Backspace => typed.pop(),
            ModalKey::Enter => {
                request.password = (!typed.is_empty()).then_some(typed);
                let req = self.safety_req();
                return Some(Modal::Confirm {
                    req,
                    change: Change::Device(request),
                    typed: String::new(),
                    error: None,
                    opened: self.now,
                });
            }
            _ => {}
        }
        Some(Modal::Password {
            request,
            typed,
            opened,
        })
    }
}
