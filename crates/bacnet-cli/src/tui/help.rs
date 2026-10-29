//! What the operator can reach and press (#1895): the screens that apply
//! to the session, for the status bar's strip, and the help overlay's
//! sections, the global keys and then the current screen's.
//!
//! A screen's section is its key table in `keymap.rs` (#1926), the rows
//! the key map dispatches from, so each description belongs to the key's
//! action. A test presses every key on every screen and checks that each
//! one the key map takes is named in that screen's help.

use super::app::command_bar::COMMANDS;
use super::app::watch::Screen;
use super::app::{App, Link};
use super::keymap::{BROWSER, Binding, CAPTURE, DEVICES, GLOBAL, MSTP, NETWORK, SC, WATCH};

/// One screen in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tab {
    /// The key that shows it.
    pub(crate) key: char,
    /// Its name.
    pub(crate) label: &'static str,
    /// It is the screen shown.
    pub(crate) current: bool,
}

/// The screens that apply to this session, in key order: Devices,
/// Browse, Watch and Capture always; Network once a session is connected
/// (its Networks tab works on every transport), SC on BACnet/SC, MS/TP
/// with a serial port. The screen shown is listed even where it
/// doesn't apply, so the strip always says where the operator is.
pub(crate) fn screens(app: &App) -> Vec<Tab> {
    let serial = app.transport == "MSTP" || app.monitor.is_some();
    // The Networks tab (routers heard) works on every transport.
    let connected = matches!(app.link, Link::Up { .. }) || app.passive_listener.is_some();
    let current = match app.screen {
        Screen::Browse => '2',
        Screen::Main => '1',
        Screen::Watch => '3',
        Screen::Network => '4',
        Screen::Sc => '5',
        Screen::Mstp => '6',
        Screen::Capture => '7',
    };
    [
        ('1', "Devices", true),
        ('2', "Browse", true),
        ('3', "Watch", true),
        ('4', "Network", connected),
        ('5', "SC", app.transport == "SC"),
        ('6', "MS/TP", serial),
        ('7', "Capture", true),
    ]
    .into_iter()
    .filter(|(key, _, applies)| *applies || *key == current)
    .map(|(key, label, _)| Tab {
        key,
        label,
        current: key == current,
    })
    .collect()
}

/// A help section: its heading and its rows, each the keys (space
/// separated, as named on the keyboard) and what they do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    /// The heading.
    pub(crate) title: &'static str,
    /// `(keys, what they do)`.
    pub(crate) rows: Vec<(String, String)>,
    /// Its keys are the screen's own (or global): false for a dialog's
    /// keys shown for reference, and for the command list.
    pub(crate) on_screen: bool,
}

impl Section {
    /// A screen's section, from the key table its key map dispatches from.
    fn keys(title: &'static str, table: &[Binding]) -> Self {
        Self {
            title,
            on_screen: true,
            rows: table
                .iter()
                .map(|row| (row.label(), row.what.to_string()))
                .collect(),
        }
    }
}

/// The Who-Is form's keys, shown for reference under the devices: the
/// form is a dialog that takes typed text, so it has no key table.
const WHO_IS: &[(&str, &str)] = &[
    ("Tab Up Down", "Next or previous field"),
    ("Left Right Space", "Change the scope"),
    ("Enter Esc", "Send (again past a warning); close"),
];

/// The help for the screen shown: the global keys, the screen's own, and
/// the command bar's commands.
pub(crate) fn sections(app: &App) -> Vec<Section> {
    let mut out = vec![Section::keys("Global", GLOBAL)];
    match app.screen {
        Screen::Browse => out.push(Section::keys("Browse (2)", BROWSER)),
        Screen::Main => {
            out.push(Section::keys("Devices (1)", DEVICES));
            out.push(Section {
                title: "Who-Is form (d)",
                on_screen: false,
                rows: WHO_IS
                    .iter()
                    .map(|(keys, what)| (keys.to_string(), what.to_string()))
                    .collect(),
            });
        }
        Screen::Watch => out.push(Section::keys("Watch (3)", WATCH)),
        Screen::Network => out.push(Section::keys("Network (4)", NETWORK)),
        Screen::Sc => out.push(Section::keys("BACnet/SC (5)", SC)),
        Screen::Mstp => out.push(Section::keys("MS/TP (6)", MSTP)),
        Screen::Capture => out.push(Section::keys("Capture (7)", CAPTURE)),
    }
    out.push(Section {
        title: "Commands (:)",
        on_screen: false,
        rows: COMMANDS
            .iter()
            .flat_map(|syntax| {
                syntax.usage.iter().map(|form| {
                    (
                        form.to_string(),
                        syntax.description(app.safety.passive).to_string(),
                    )
                })
            })
            .collect(),
    });
    out
}
