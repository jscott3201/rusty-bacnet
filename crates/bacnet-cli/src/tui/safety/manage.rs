//! Device management from the TUI (#1799): DeviceCommunicationControl
//! (`:dcc`) and ReinitializeDevice (`:reinit`), each to one device.
//!
//! Both are the one-shot guard's device-management class: they need an
//! armed write session on a profile with `dangerous = true`, and always
//! take the T3 confirmation (the device instance typed out). The password,
//! when one is given, comes from a masked prompt and lives only in a
//! [`Secret`]: the audit log, the screen and the logs never see its text.
//! Neither service is kept in the ledger, since neither can be undone by
//! writing an old value back.

use bacnet_types::enums::{EnableDisable, ReinitializedState};
use serde_json::{Map, Value, json};

use super::audit::Secret;
use super::{Gate, Tier};
use crate::commands::device::{DCC_ACTIONS, REINIT_STATES};

/// The longest password either service carries, in characters
/// (Clauses 16.1.1.1.3 and 16.4.1.1.2).
pub(crate) const MAX_PASSWORD: usize = 20;

/// Why a profile without the dangerous opt-in may not send either service.
pub(crate) const NEEDS_DANGEROUS: &str = "device management (dcc, reinit) needs a profile with \
     dangerous = true in its [safety] table";

/// A disable's length when none is typed, in minutes (decided on #1799).
pub(crate) const DEFAULT_DCC_MINUTES: u16 = 15;

/// How long a DeviceCommunicationControl lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DccTime {
    /// None typed: [`DEFAULT_DCC_MINUTES`].
    Default,
    /// The minutes typed, 1 to 65535.
    Minutes(u16),
    /// `forever`, or an `enable`: no time duration is sent, so a disable
    /// lasts until enabled again or a restart.
    Forever,
}

impl DccTime {
    /// The duration sent, in minutes; `None` sends none.
    pub(crate) fn minutes(self) -> Option<u16> {
        match self {
            Self::Default => Some(DEFAULT_DCC_MINUTES),
            Self::Minutes(minutes) => Some(minutes),
            Self::Forever => None,
        }
    }
}

/// What to ask the device to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// DeviceCommunicationControl.
    Dcc {
        /// Enable, disable, or disable initiation.
        state: EnableDisable,
        /// How long.
        time: DccTime,
    },
    /// ReinitializeDevice.
    Reinit {
        /// Which restart or backup step.
        state: ReinitializedState,
    },
}

/// `value`'s name in `table`, or its own display (a number for one
/// without a name).
fn name_in<T: Copy + PartialEq + std::fmt::Display>(
    table: &[(&'static str, T)],
    value: T,
) -> String {
    table
        .iter()
        .find(|(_, v)| *v == value)
        .map_or_else(|| value.to_string(), |(name, _)| (*name).to_string())
}

impl Action {
    /// The service's name.
    pub(crate) fn service(self) -> &'static str {
        match self {
            Self::Dcc { .. } => "DeviceCommunicationControl",
            Self::Reinit { .. } => "ReinitializeDevice",
        }
    }

    /// The command bar word, which is also the audit event.
    pub(crate) fn command(self) -> &'static str {
        match self {
            Self::Dcc { .. } => "dcc",
            Self::Reinit { .. } => "reinit",
        }
    }

    /// The state as the command bar names it.
    pub(crate) fn state(self) -> String {
        match self {
            Self::Dcc { state, .. } => name_in(&DCC_ACTIONS, state),
            Self::Reinit { state } => name_in(&REINIT_STATES, state),
        }
    }

    /// `disable for 30 min`, `disable for 15 min (default)`, `disable,
    /// no time limit`, `enable`, `coldstart`.
    pub(crate) fn describe(self) -> String {
        let state = self.state();
        match self {
            Self::Dcc {
                state: EnableDisable::ENABLE,
                ..
            }
            | Self::Reinit { .. } => state,
            Self::Dcc {
                time: DccTime::Default,
                ..
            } => format!("{state} for {DEFAULT_DCC_MINUTES} min (default)"),
            Self::Dcc {
                time: DccTime::Minutes(minutes),
                ..
            } => format!("{state} for {minutes} min"),
            Self::Dcc {
                time: DccTime::Forever,
                ..
            } => format!("{state}, no time limit"),
        }
    }
}

/// One device management request, ready to confirm and send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The device instance.
    pub(crate) device: u32,
    /// What to ask.
    pub(crate) action: Action,
    /// The password; `None` sends none.
    pub(crate) password: Option<Secret>,
}

impl Request {
    /// `DeviceCommunicationControl to device 100: disable for 30 min`.
    pub(crate) fn label(&self) -> String {
        format!(
            "{} to device {}: {}",
            self.action.service(),
            self.device,
            self.action.describe()
        )
    }

    /// Always T3.
    pub(crate) fn gate(&self) -> Gate {
        Gate::Allowed(Tier::T3)
    }

    /// The audit fields: service, target, parameters and tier. A password
    /// shows only as present, its value written as `[redacted]`.
    pub(crate) fn audit_json(&self) -> Value {
        let mut params = Map::new();
        params.insert("state".into(), json!(self.action.state()));
        if let Action::Dcc { time, .. } = self.action {
            params.insert("minutes".into(), json!(time.minutes()));
            if time == DccTime::Default {
                params.insert("default_duration".into(), json!(true));
            }
        }
        if let Some(password) = &self.password {
            params.insert("password".into(), json!(password));
        }
        json!({
            "service": self.action.service(),
            "target": { "device": self.device },
            "params": params,
            "tier": "T3",
        })
    }

    /// The request without its password, for a decline: what was declined
    /// is recorded, and the password goes no further.
    pub(crate) fn forget_password(&mut self) {
        self.password = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_their_service_state_and_device() {
        let dcc = Request {
            device: 100,
            action: Action::Dcc {
                state: EnableDisable::DISABLE_INITIATION,
                time: DccTime::Minutes(30),
            },
            password: Some(Secret::new("hunter2-unit")),
        };
        assert_eq!(
            dcc.label(),
            "DeviceCommunicationControl to device 100: disable-initiation for 30 min"
        );
        let reinit = Request {
            device: 7,
            action: Action::Reinit {
                state: ReinitializedState::ACTIVATE_CHANGES,
            },
            password: None,
        };
        assert_eq!(
            reinit.label(),
            "ReinitializeDevice to device 7: activate-changes"
        );
        let dcc = |state, time| Action::Dcc { state, time }.describe();
        assert_eq!(
            dcc(EnableDisable::DISABLE, DccTime::Default),
            "disable for 15 min (default)"
        );
        assert_eq!(
            dcc(EnableDisable::DISABLE, DccTime::Forever),
            "disable, no time limit"
        );
        assert_eq!(dcc(EnableDisable::ENABLE, DccTime::Forever), "enable");
        assert_eq!(DccTime::Default.minutes(), Some(DEFAULT_DCC_MINUTES));
        assert_eq!(DccTime::Forever.minutes(), None);
        // A state without a name shows its number.
        let odd = Action::Reinit {
            state: ReinitializedState::from_raw(99),
        };
        assert_eq!(odd.state(), "99");
    }

    /// Neither Debug output nor the audit fields hold the password text.
    #[test]
    fn the_password_shows_only_as_present() {
        let mut dcc = Request {
            device: 100,
            action: Action::Dcc {
                state: EnableDisable::DISABLE,
                time: DccTime::Forever,
            },
            password: Some(Secret::new("hunter2-unit")),
        };
        let debug = format!("{dcc:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        let fields = dcc.audit_json();
        assert!(!fields.to_string().contains("hunter2"), "{fields}");
        assert_eq!(fields["params"]["password"], "[redacted]");
        assert_eq!(fields["params"]["minutes"], Value::Null);
        assert_eq!(fields["params"]["state"], "disable");
        dcc.forget_password();
        assert!(dcc.audit_json()["params"].get("password").is_none());
    }
}
