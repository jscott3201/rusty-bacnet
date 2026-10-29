//! Write sessions in the app: arming (`W`), the write and relinquish forms
//! (`w`, `n` on an open object), the preview and its confirmation tier,
//! the ledger (`u`) and the exit guard. The device management requests of
//! `:dcc` and `:reinit` (in [`super::manage`]) share the confirmation.
//!
//! The app decides what the user may ask for; the write worker checks the
//! same rules again before it sends anything and records every outcome.

use std::time::{Duration, Instant};

use bacnet_ops::class::Class;
use bacnet_ops::parse::{parse_property, parse_value_for};
use bacnet_types::enums::PropertyIdentifier;

use super::browser::ViewState;
use super::{App, FLASH_FOR, Link, Overlay};
use crate::tui::message::Command;
use crate::tui::point::BrowseRead;
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::audit::Secret;
use crate::tui::safety::ledger::Entry;
use crate::tui::safety::manage::Request;
use crate::tui::safety::{
    Change, DEFAULT_MINUTES, Gate, Level, MAX_MINUTES, Outcome, SafetyCommand, SafetyEvent, Tier,
    WriteSpec,
};

/// How long after a preview opens its confirmation is ignored, so a
/// repeating key can't confirm a write nobody has seen.
pub(crate) const CONFIRM_GUARD: Duration = Duration::from_millis(300);

/// How long a preview may wait for its confirmation before it is
/// declined as stale.
pub(crate) const PREVIEW_LIFETIME: Duration = Duration::from_secs(120);

/// What `w` says in a read-only session.
pub(crate) const ARM_HINT: &str = "Read-only: press W to arm a write session first.";

/// An armed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Armed {
    /// When it disarms.
    pub(crate) until: Instant,
    /// The reason given.
    pub(crate) reason: String,
}

/// The app's write session state.
#[derive(Debug, Default)]
pub(crate) struct SafetyState {
    /// `--passive`.
    pub(crate) passive: bool,
    /// The profile's policy, once the worker has opened the session.
    pub(crate) policy: Option<Safety>,
    /// A workspace holds the ledger and the audit log.
    pub(crate) workspace: bool,
    /// This process's run, marking the ledger entries it wrote.
    pub(crate) run: String,
    /// The armed session.
    pub(crate) armed: Option<Armed>,
    /// The ledger as the worker last reported it.
    pub(crate) ledger: Vec<Entry>,
    /// The reason typed in the arm form, until the worker confirms.
    pending_reason: String,
    /// A Prepare, Execute or Revert the worker hasn't answered yet; the
    /// exit guard waits for it as for a ledger entry.
    pub(crate) pending: Option<u64>,
    next_req: u64,
    /// Revert all, then quit.
    quitting: bool,
    last_second: u64,
}

impl SafetyState {
    /// Read-only, or passive with `--passive`.
    pub(crate) fn new(passive: bool) -> Self {
        Self {
            passive,
            ..Self::default()
        }
    }

    /// The level for the status bar.
    pub(crate) fn level(&self) -> Level {
        match (&self.armed, &self.policy) {
            _ if self.passive => Level::Passive,
            (Some(_), policy) => Level::Armed {
                dangerous: policy.as_ref().is_some_and(|p| p.dangerous),
            },
            (None, _) => Level::ReadOnly,
        }
    }

    /// Ledger entries left by an earlier run.
    pub(crate) fn leftovers(&self) -> usize {
        self.ledger.iter().filter(|e| e.run != self.run).count()
    }
}

/// A key in a write session dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalKey {
    /// A typed character.
    Char(char),
    /// Erase.
    Backspace,
    /// Next field.
    Next,
    /// Previous field.
    Prev,
    /// Submit or confirm.
    Enter,
    /// Close, declining.
    Esc,
}

/// Labelled text fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Form {
    /// `(label, text)`.
    pub(crate) fields: Vec<(&'static str, String)>,
    /// The focused field.
    pub(crate) focus: usize,
    /// Why the last submit failed.
    pub(crate) error: Option<String>,
}

impl Form {
    fn new(fields: Vec<(&'static str, String)>) -> Self {
        Self {
            fields,
            focus: 0,
            error: None,
        }
    }

    fn get(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|(l, _)| *l == label)
            .map_or("", |(_, text)| text.trim())
    }

    /// Edit; true on Enter.
    fn key(&mut self, key: ModalKey) -> bool {
        let count = self.fields.len();
        match key {
            ModalKey::Char(ch) if self.fields[self.focus].1.len() < 128 => {
                self.fields[self.focus].1.push(ch)
            }
            ModalKey::Backspace => {
                self.fields[self.focus].1.pop();
            }
            ModalKey::Next => self.focus = (self.focus + 1) % count,
            ModalKey::Prev => self.focus = (self.focus + count - 1) % count,
            ModalKey::Enter => return true,
            _ => {}
        }
        false
    }
}

/// A write session dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Modal {
    /// Arm: a reason and a duration.
    Arm(Form),
    /// Write (or relinquish, with only a priority) to an object.
    Write {
        /// Device instance.
        device: u32,
        /// The object.
        object: bacnet_types::primitives::ObjectIdentifier,
        /// Relinquish rather than write.
        relinquish: bool,
        /// The fields.
        form: Form,
    },
    /// Waiting for the worker.
    Waiting {
        /// The request.
        req: u64,
        /// What for.
        what: &'static str,
    },
    /// The masked password prompt of a device management request.
    Password {
        /// The request, its password not yet set.
        request: Box<Request>,
        /// What has been typed, shown only as a mask.
        typed: Secret,
        /// When it opened: like a preview, it closes after
        /// [`PREVIEW_LIFETIME`], and what was typed goes with it.
        opened: Instant,
    },
    /// A change to confirm at its tier.
    Confirm {
        /// The request.
        req: u64,
        /// The write preview or the device management request.
        change: Change,
        /// The device instance typed for T3.
        typed: String,
        /// Why the last confirm failed.
        error: Option<String>,
        /// When it opened: Enter is ignored for a moment after, and the
        /// preview expires after [`PREVIEW_LIFETIME`].
        opened: Instant,
    },
    /// How the writes went.
    Result(Vec<(String, Outcome)>),
    /// The ledger; `confirm` once R asks to revert everything.
    Ledger {
        /// A note at the top (entries from an earlier run, a revert).
        notice: Option<String>,
        /// R was pressed: y reverts.
        confirm: bool,
    },
    /// Quitting with entries outstanding.
    ExitGuard(Option<String>),
}

pub(super) fn modal(modal: Modal) -> Overlay {
    Overlay::Safety(Box::new(modal))
}

impl App {
    /// The level for the status bar.
    pub(crate) fn level(&self) -> Level {
        self.safety.level()
    }

    pub(super) fn safety_req(&mut self) -> u64 {
        self.safety.next_req += 1;
        self.safety.next_req
    }

    /// False, with a message, in a passive session.
    pub(super) fn may_transmit(&mut self) -> bool {
        if self.safety.passive {
            self.flash("Passive: this session transmits nothing.", FLASH_FOR);
        }
        !self.safety.passive
    }

    /// `W`: arm, or disarm an armed session.
    pub(super) fn toggle_arm(&mut self, out: &mut Vec<Command>) {
        if self.safety.armed.take().is_some() {
            out.push(Command::Safety(SafetyCommand::Disarm { expired: false }));
            self.flash("Disarmed: read-only.", FLASH_FOR);
            return;
        }
        if !self.may_transmit() {
            return;
        }
        let why = match &self.safety.policy {
            None => Some("Not connected: a write session needs a session to arm."),
            Some(policy) if policy.writes == Writes::Never => {
                Some("Writes are disabled: this profile sets writes = \"never\".")
            }
            Some(_) if !self.safety.workspace => {
                Some("Write sessions need a profile: its workspace holds the ledger and audit log.")
            }
            Some(_) => None,
        };
        match why {
            Some(why) => self.flash(why, FLASH_FOR),
            None => {
                self.overlay = modal(Modal::Arm(Form::new(vec![
                    ("Reason", String::new()),
                    ("Minutes", DEFAULT_MINUTES.to_string()),
                ])))
            }
        }
    }

    /// `w` or `n` on an open object.
    pub(super) fn open_write(&mut self, relinquish: bool) {
        if !self.may_transmit() {
            return;
        }
        if !self.level().permits(Class::Write) {
            self.flash(ARM_HINT, FLASH_FOR);
            return;
        }
        let Some(browser) = self.shown_browser() else {
            self.flash("Open a device and an object first.", FLASH_FOR);
            return;
        };
        let ViewState::Ready(view) = &browser.view else {
            self.flash("Read an object first (Enter).", FLASH_FOR);
            return;
        };
        let (device, object) = (browser.device, view.object);
        let ours = self
            .safety
            .ledger
            .iter()
            .find(|e| e.device == device && e.object() == Some(object) && e.priority.is_some());
        let winner = view.priority.as_ref().and_then(|p| p.winner());
        let priority = match (relinquish, ours, &view.priority) {
            (true, Some(entry), _) => entry.priority.map(|p| p.to_string()),
            (true, None, _) => winner.map(|p| p.to_string()),
            (false, _, Some(_)) => Some("16".into()),
            (false, _, None) => None,
        };
        let mut fields = Vec::new();
        if !relinquish {
            fields.push(("Property", "present-value".to_string()));
            fields.push(("Value", String::new()));
        }
        fields.push(("Priority", priority.unwrap_or_default()));
        let mut form = Form::new(fields);
        form.focus = usize::from(!relinquish);
        self.overlay = modal(Modal::Write {
            device,
            object,
            relinquish,
            form,
        });
    }

    /// `u`: the ledger.
    pub(super) fn open_ledger(&mut self) {
        self.overlay = modal(Modal::Ledger {
            notice: None,
            confirm: false,
        });
    }

    /// Quit, through the exit guard when the ledger holds changes or a
    /// write session request is still out.
    pub(super) fn request_quit(&mut self) {
        let guarding =
            matches!(&self.overlay, Overlay::Safety(m) if matches!(**m, Modal::ExitGuard(_)));
        let open = !self.safety.ledger.is_empty() || self.safety.pending.is_some();
        if !open || guarding {
            self.quit = true;
        } else {
            self.overlay = modal(Modal::ExitGuard(None));
        }
    }

    /// Start a write or a batch the command bar parsed.
    pub(super) fn prepare(&mut self, specs: Vec<WriteSpec>, batch: bool, out: &mut Vec<Command>) {
        if !self.may_transmit() {
            return;
        }
        if !self.level().permits(Class::Write) {
            self.flash(ARM_HINT, FLASH_FOR);
            return;
        }
        let req = self.safety_req();
        self.safety.pending = Some(req);
        out.push(Command::Safety(SafetyCommand::Prepare {
            req,
            specs,
            batch,
        }));
        self.overlay = modal(Modal::Waiting {
            req,
            what: "Reading the priority array and the old values...",
        });
    }

    /// Once a second: decline a preview left too long, close a password
    /// prompt left too long (dropping what was typed), disarm at expiry,
    /// and redraw the countdown.
    pub(super) fn safety_tick(&mut self, out: &mut Vec<Command>) {
        let prompt = matches!(&self.overlay, Overlay::Safety(m)
            if matches!(**m, Modal::Password { opened, .. }
                if self.now.saturating_duration_since(opened) >= PREVIEW_LIFETIME));
        if prompt {
            self.overlay = Overlay::None;
            self.flash(
                "The password prompt expired: nothing was sent, and the password was cleared.",
                FLASH_FOR,
            );
        }
        let stale = matches!(&self.overlay, Overlay::Safety(m)
            if matches!(**m, Modal::Confirm { opened, .. }
                if self.now.saturating_duration_since(opened) >= PREVIEW_LIFETIME));
        if stale {
            self.modal_key(ModalKey::Esc, out);
            self.flash(
                "The preview expired unconfirmed: nothing was sent.",
                FLASH_FOR,
            );
        }
        let Some(armed) = &self.safety.armed else {
            return;
        };
        if self.now >= armed.until {
            self.safety.armed = None;
            out.push(Command::Safety(SafetyCommand::Disarm { expired: true }));
            self.flash("The write session expired: read-only again.", FLASH_FOR);
            self.dirty = true;
            return;
        }
        let left = armed.until.saturating_duration_since(self.now).as_secs();
        if left != self.safety.last_second {
            self.safety.last_second = left;
            self.dirty = true;
        }
    }

    /// A key in a write session dialog.
    pub(super) fn modal_key(&mut self, key: ModalKey, out: &mut Vec<Command>) {
        let Overlay::Safety(current) = std::mem::replace(&mut self.overlay, Overlay::None) else {
            return;
        };
        let next = match *current {
            Modal::Arm(mut form) => match key {
                ModalKey::Esc => None,
                _ if form.key(key) => match form.get("Minutes").parse::<u32>() {
                    Ok(minutes) if (1..=MAX_MINUTES).contains(&minutes) => {
                        self.safety.pending_reason = form.get("Reason").to_string();
                        out.push(Command::Safety(SafetyCommand::Arm {
                            reason: self.safety.pending_reason.clone(),
                            minutes,
                        }));
                        None
                    }
                    _ => {
                        form.error = Some(format!("Minutes: 1 to {MAX_MINUTES}."));
                        Some(Modal::Arm(form))
                    }
                },
                _ => Some(Modal::Arm(form)),
            },
            Modal::Write {
                device,
                object,
                relinquish,
                mut form,
            } => match key {
                ModalKey::Esc => None,
                _ if form.key(key) => match write_spec(device, object, relinquish, &form) {
                    Ok(spec) => {
                        self.prepare(vec![spec], false, out);
                        return;
                    }
                    Err(error) => {
                        form.error = Some(error);
                        Some(Modal::Write {
                            device,
                            object,
                            relinquish,
                            form,
                        })
                    }
                },
                _ => Some(Modal::Write {
                    device,
                    object,
                    relinquish,
                    form,
                }),
            },
            Modal::Password {
                request,
                typed,
                opened,
            } => self.password_key(key, request, typed, opened),
            Modal::Confirm {
                req,
                change,
                typed,
                error,
                opened,
            } => self.confirm_key(key, req, change, (typed, error, opened), out),
            Modal::Ledger { confirm, notice } => match key {
                ModalKey::Char('R') if !self.safety.ledger.is_empty() => Some(Modal::Ledger {
                    notice,
                    confirm: true,
                }),
                ModalKey::Char('y') if confirm => self.revert(out),
                ModalKey::Esc | ModalKey::Char('q') => None,
                _ => Some(Modal::Ledger { notice, confirm }),
            },
            Modal::ExitGuard(error) => match key {
                ModalKey::Char('r') if !matches!(self.link, Link::Up { .. }) => Some(
                    Modal::ExitGuard(Some("Not connected: reverting needs the network.".into())),
                ),
                ModalKey::Char('r') => {
                    self.safety.quitting = true;
                    self.revert(out)
                }
                ModalKey::Char('k') => {
                    out.push(Command::Safety(SafetyCommand::Keep));
                    self.quit = true;
                    None
                }
                ModalKey::Esc | ModalKey::Char('c') => None,
                _ => Some(Modal::ExitGuard(error)),
            },
            other @ (Modal::Waiting { .. } | Modal::Result(_)) => match key {
                ModalKey::Esc | ModalKey::Enter => None,
                _ => Some(other),
            },
        };
        if let Some(next) = next {
            self.overlay = modal(next);
        }
    }

    fn revert(&mut self, out: &mut Vec<Command>) -> Option<Modal> {
        if !self.may_transmit() {
            return None;
        }
        let req = self.safety_req();
        self.safety.pending = Some(req);
        out.push(Command::Safety(SafetyCommand::Revert { req }));
        Some(Modal::Waiting {
            req,
            what: "Reverting, newest first...",
        })
    }

    fn confirm_key(
        &mut self,
        key: ModalKey,
        req: u64,
        change: Change,
        (mut typed, error, opened): (String, Option<String>, Instant),
        out: &mut Vec<Command>,
    ) -> Option<Modal> {
        let fresh = self.now.saturating_duration_since(opened) < CONFIRM_GUARD;
        let device = change.device();
        let confirmed = match (&change.gate(), key) {
            (_, ModalKey::Esc) => {
                declined(change, out);
                self.flash("Declined: nothing was sent.", FLASH_FOR);
                return None;
            }
            // Too soon after it opened to have been read.
            (_, ModalKey::Enter | ModalKey::Char('y')) if fresh => false,
            (Gate::Allowed(Tier::T1), ModalKey::Enter) => true,
            (Gate::Allowed(Tier::T2), ModalKey::Char('y')) => true,
            (Gate::Allowed(Tier::T3), ModalKey::Char(ch)) if ch.is_ascii_digit() => {
                typed.push(ch);
                false
            }
            (Gate::Allowed(Tier::T3), ModalKey::Backspace) => {
                typed.pop();
                false
            }
            (Gate::Allowed(Tier::T3), ModalKey::Enter) => {
                if typed == device.to_string() {
                    true
                } else {
                    let error = format!("Type the device instance, {device}, to confirm.");
                    return Some(Modal::Confirm {
                        req,
                        change,
                        typed: String::new(),
                        error: Some(error),
                        opened,
                    });
                }
            }
            _ => false,
        };
        if !confirmed {
            return Some(Modal::Confirm {
                req,
                change,
                typed,
                error,
                opened,
            });
        }
        if self.safety.armed.is_none() {
            self.flash(
                "The write session has expired; nothing was sent.",
                FLASH_FOR,
            );
            declined(change, out);
            return None;
        }
        self.safety.pending = Some(req);
        let (command, what) = match change {
            Change::Write(preview) => (SafetyCommand::Execute { req, preview }, "Writing..."),
            Change::Device(request) => (SafetyCommand::Manage { req, request }, "Sending..."),
        };
        out.push(Command::Safety(command));
        Some(Modal::Waiting { req, what })
    }

    /// A write session report.
    pub(super) fn safety_event(&mut self, event: SafetyEvent, out: &mut Vec<Command>) {
        let waiting = |app: &App, id: u64| matches!(&app.overlay, Overlay::Safety(m) if matches!(**m, Modal::Waiting { req, .. } if req == id));
        let answered = match &event {
            SafetyEvent::Preview { req, .. }
            | SafetyEvent::Done { req, .. }
            | SafetyEvent::Reverted { req, .. }
            | SafetyEvent::Refused { req: Some(req), .. } => Some(*req),
            _ => None,
        };
        if answered.is_some() && answered == self.safety.pending {
            self.safety.pending = None;
        }
        match event {
            SafetyEvent::Ready {
                policy,
                workspace,
                run,
                ledger,
            } => {
                self.safety.policy = Some(policy);
                self.safety.workspace = workspace;
                self.safety.run = run;
                self.safety.ledger = ledger;
                let left = self.safety.leftovers();
                if left > 0 && self.overlay == Overlay::None {
                    let notice = format!(
                        "{left} change(s) left by an earlier session are still recorded. Revert them (R) or check the devices."
                    );
                    self.overlay = modal(Modal::Ledger {
                        notice: Some(notice),
                        confirm: false,
                    });
                }
            }
            SafetyEvent::Armed { minutes } => {
                self.safety.armed = Some(Armed {
                    until: self.now + Duration::from_secs(u64::from(minutes) * 60),
                    reason: std::mem::take(&mut self.safety.pending_reason),
                });
                self.flash(format!("Write session armed for {minutes} min."), FLASH_FOR);
            }
            SafetyEvent::Disarmed => self.safety.armed = None,
            SafetyEvent::Refused { req, reason } => {
                if req.is_none_or(|req| waiting(self, req)) {
                    if self.safety.quitting {
                        self.safety.quitting = false;
                        self.overlay = modal(Modal::ExitGuard(Some(reason.clone())));
                    } else if req.is_some() {
                        self.overlay = Overlay::None;
                    }
                }
                self.flash(format!("Refused: {reason}"), FLASH_FOR);
            }
            SafetyEvent::Preview { req, preview } => {
                if waiting(self, req) {
                    self.overlay = modal(Modal::Confirm {
                        req,
                        change: Change::Write(preview),
                        typed: String::new(),
                        error: None,
                        opened: self.now,
                    });
                }
            }
            SafetyEvent::Done { req, outcomes } => {
                if waiting(self, req) {
                    self.overlay = modal(Modal::Result(outcomes));
                } else if let Some((label, outcome)) = outcomes.first() {
                    // The view was closed: say how it went anyway.
                    self.flash(format!("{label}: {}", outcome.describe()), FLASH_FOR);
                }
                self.reread_object(out);
            }
            SafetyEvent::Ledger(ledger) => self.safety.ledger = ledger,
            SafetyEvent::Reverted { req, failed } => {
                if !waiting(self, req) {
                    return;
                }
                self.reread_object(out);
                if std::mem::take(&mut self.safety.quitting) {
                    if failed == 0 {
                        self.quit = true;
                        return;
                    }
                    let error = format!("{failed} change(s) could not be reverted.");
                    self.overlay = modal(Modal::ExitGuard(Some(error)));
                } else {
                    let notice = format!("Reverted; {failed} change(s) left.");
                    self.overlay = modal(Modal::Ledger {
                        notice: Some(notice),
                        confirm: false,
                    });
                }
            }
        }
    }

    /// Read the open object again, so its priority pane shows the change.
    fn reread_object(&mut self, out: &mut Vec<Command>) {
        let req = self.next_req();
        if let Some(browser) = self.browser.as_deref_mut()
            && let ViewState::Ready(view) = &browser.view
        {
            let object = view.object;
            browser.reading(req, object);
            out.push(Command::Browse {
                req,
                device: browser.device,
                read: BrowseRead::Object(object),
            });
        }
    }
}

/// Record a declined change, a device management request without its
/// password: the password goes no further than the dialog.
fn declined(mut change: Change, out: &mut Vec<Command>) {
    if let Change::Device(request) = &mut change {
        request.forget_password();
    }
    out.push(Command::Safety(SafetyCommand::Declined { change }));
}

/// The form's write, checked with the one-shot CLI's parsers.
fn write_spec(
    device: u32,
    object: bacnet_types::primitives::ObjectIdentifier,
    relinquish: bool,
    form: &Form,
) -> Result<WriteSpec, String> {
    let (property, index) = if relinquish {
        (PropertyIdentifier::PRESENT_VALUE, None)
    } else {
        parse_property(form.get("Property"))?
    };
    let value = if relinquish {
        None
    } else {
        let text = form.get("Value");
        parse_value_for(object.object_type(), property, index, text)?;
        Some(text.to_string())
    };
    let priority = match form.get("Priority") {
        "" if relinquish => return Err("Priority: the slot to relinquish, 1-16.".into()),
        "" => None,
        text => match text.parse::<u8>() {
            Ok(p) if (1..=16).contains(&p) => Some(p),
            _ => {
                return Err(
                    "Priority: 1-16, or blank for a property that isn't commandable.".into(),
                );
            }
        },
    };
    Ok(WriteSpec {
        device,
        object,
        property,
        index,
        value,
        priority,
    })
}
