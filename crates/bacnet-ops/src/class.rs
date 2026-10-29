//! What each command does to a device: read, write or device management
//! (#972). One table serves the one-shot commands and the TUI's safety
//! levels (#964), so a command can't be guarded in one front end and not in
//! another.
//!
//! - **Read**: discovery, reads, finite COV subscriptions and
//!   foreign-device registration, plus the commands that never reach a
//!   device (`mstp-ports`, `capture`, `sc-check`, `help`), and `tui` and
//!   `mcp`, which guard their own writes. A
//!   subscription and a registration do leave an entry on the remote side,
//!   but only to deliver what the client watches, and it lapses on its own:
//!   `subscribe` cancels its subscription when it stops, and a registration
//!   runs out with its TTL. The TUI makes both from its read-only level.
//! - **Write**: anything that changes a value, an object, a file, an
//!   alarm's acknowledgment, the device's clock or a BBMD table, and a
//!   subscription that never expires ([`of_subscribe`]).
//! - **Device management**: communication control and reinitialize, which
//!   change whether and how the device runs at all.

use serde::Serialize;

/// What a command does to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    /// Observes only.
    Read,
    /// Changes a value, an object, a file or a table.
    Write,
    /// Changes whether or how the device runs.
    DeviceManagement,
}

impl Class {
    /// The name in messages and JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::DeviceManagement => "device-management",
        }
    }
}

use Class::{DeviceManagement, Read, Write};

/// Every subcommand, aliases included, with its class. A test in
/// `bacnet-cli` checks this against the clap definitions in both
/// directions.
pub const COMMANDS: &[(&str, Class)] = &[
    // Discovery and reads.
    ("discover", Read),
    ("whois", Read),
    ("find", Read),
    ("whohas", Read),
    ("read", Read),
    ("rp", Read),
    ("readm", Read),
    ("rpm", Read),
    ("read-range", Read),
    ("rr", Read),
    ("alarms", Read),
    ("alarm-summary", Read),
    ("enrollment-summary", Read),
    ("file-read", Read),
    ("bdt", Read),
    ("fdt", Read),
    ("whois-router", Read),
    ("subscribe", Read),
    ("cov", Read),
    ("register", Read),
    // Nothing reaches a device, or a front end whose own writes are
    // guarded one by one.
    ("mstp-ports", Read),
    ("capture", Read),
    ("sc-check", Read),
    ("tui", Read),
    ("mcp", Read),
    ("help", Read),
    // Writes.
    ("write", Write),
    ("wp", Write),
    ("writem", Write),
    ("wpm", Write),
    ("create-object", Write),
    ("delete-object", Write),
    ("file-write", Write),
    ("ack-alarm", Write),
    ("ack", Write),
    ("time-sync", Write),
    ("ts", Write),
    // Delete-Foreign-Device-Table-Entry: the CLI names its own address,
    // but the BBMD removes whichever entry has it.
    ("unregister", Write),
    // Device management.
    ("control", DeviceManagement),
    ("dcc", DeviceManagement),
    ("reinit", DeviceManagement),
];

/// The class of `subscribe` (`cov`) asking for `lifetime` seconds. A
/// finite subscription is renewed while the CLI watches and cancelled when
/// it stops, so it is a read; one of 0 never expires (Clause 13.14.1.1.4)
/// and outlives the CLI, so it is a write.
pub fn of_subscribe(lifetime: u32) -> Class {
    if lifetime == 0 { Write } else { Read }
}

/// The class of a subcommand, by name or alias;
/// `None` for a word that is neither.
pub fn of(command: &str) -> Option<Class> {
    COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|&(_, class)| class)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classes themselves, pinned: a change here is a safety change.
    #[test]
    fn the_mutating_commands_are_pinned() {
        let mutating: Vec<(&str, &str)> = COMMANDS
            .iter()
            .filter(|(_, class)| *class != Read)
            .map(|&(name, class)| (name, class.name()))
            .collect();
        assert_eq!(
            mutating,
            [
                ("write", "write"),
                ("wp", "write"),
                ("writem", "write"),
                ("wpm", "write"),
                ("create-object", "write"),
                ("delete-object", "write"),
                ("file-write", "write"),
                ("ack-alarm", "write"),
                ("ack", "write"),
                ("time-sync", "write"),
                ("ts", "write"),
                ("unregister", "write"),
                ("control", "device-management"),
                ("dcc", "device-management"),
                ("reinit", "device-management"),
            ]
        );
        assert_eq!(of("subscribe"), Some(Read));
        assert_eq!(of_subscribe(300), Read);
        assert_eq!(of_subscribe(1), Read);
        assert_eq!(of_subscribe(0), Write);
        assert_eq!(of("register"), Some(Read));
        assert_eq!(of("mcp"), Some(Read));
        assert_eq!(of("no-such-command"), None);
    }
}
