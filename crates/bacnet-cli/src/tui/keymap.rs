//! Keys to [`Action`]s, per context.
//!
//! Text fields take printable characters, so `q` types a `q` in the filter or
//! the Who-Is form; Ctrl-C works everywhere.
//!
//! Each screen's keys are one table of [`Binding`]s (#1926): the key map
//! dispatches from it and the help overlay ([`crate::tui::help`]) prints
//! it, so what help says a key does is what the key does.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::capture::{CaptureKey, Focus};
use super::app::devices::Movement;
use super::app::mstp::{JoinKey, MstpKey};
use super::app::network::NetworkKey;
use super::app::picker::PickerKey;
use super::app::safety::ModalKey;
use super::app::sc::ScKey;
use super::app::watch::Screen;
use super::app::whois::FormKey;
use super::app::{Action, App, FilterKey, Overlay};

/// Map a key press to an action, or `None` to ignore it.
pub(crate) fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    if let Some(action) = chord(GLOBAL, key) {
        return Some(action);
    }
    match &app.overlay {
        Overlay::Connection => connection(app, key),
        Overlay::Picker(_) => picker(key),
        Overlay::WhoIs(_) => form(key),
        Overlay::Help => help(key),
        Overlay::Command(_) => command_line(key),
        Overlay::Safety(_) => safety(key),
        Overlay::Join(_) => join(key),
        Overlay::None if app.filter_editing => filter(key),
        Overlay::None if app.screen == Screen::Watch => watch(key),
        Overlay::None if app.screen == Screen::Network => network(key),
        Overlay::None if app.screen == Screen::Sc => sc(key),
        Overlay::None if app.screen == Screen::Mstp => mstp(key),
        Overlay::None if app.screen == Screen::Capture => capture(app, key),
        Overlay::None if app.screen == Screen::Browse => browse(key),
        Overlay::None => main(key),
    }
}

/// A printable character typed without Ctrl or Alt.
fn typed(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(ch)
        }
        _ => None,
    }
}

/// The movement a key makes in a list, as [`MOVE`], [`JUMP`] and [`ENDS`]
/// bind it.
fn movement(code: KeyCode) -> Option<Movement> {
    match lookup(&[MOVE, JUMP, ENDS], code)? {
        Action::Move(movement) => Some(movement),
        _ => None,
    }
}

/// A printable character typed with Ctrl or Alt, which no screen takes.
fn chorded(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_)) && typed(key).is_none()
}

/// One row of a screen's key table: its keys, what each one does, and
/// what help says about them.
#[derive(Debug)]
pub(crate) struct Binding {
    /// Each key and its action, in the order help names them.
    pub(crate) keys: &'static [(KeyCode, Action)],
    /// The keys are pressed with Ctrl (help shows `Ctrl-C`).
    pub(crate) ctrl: bool,
    /// What help says the keys do.
    pub(crate) what: &'static str,
}

/// A row of plain keys.
const fn keys(keys: &'static [(KeyCode, Action)], what: &'static str) -> Binding {
    Binding {
        keys,
        ctrl: false,
        what,
    }
}

impl Binding {
    /// The keys as help names them, space separated: `Tab Shift-Tab`.
    pub(crate) fn label(&self) -> String {
        let names: Vec<String> = self
            .keys
            .iter()
            .map(|(code, _)| match (self.ctrl, code) {
                (true, KeyCode::Char(ch)) => format!("Ctrl-{}", ch.to_ascii_uppercase()),
                _ => key_name(*code),
            })
            .collect();
        names.join(" ")
    }
}

/// A key as help names it: `Space`, `Shift-Tab`, `PgDn`, `q`.
pub(crate) fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::BackTab => "Shift-Tab".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        other => format!("{other:?}"),
    }
}

/// The action `table`'s plain keys bind to `code`.
fn lookup(table: &[Binding], code: KeyCode) -> Option<Action> {
    table
        .iter()
        .filter(|row| !row.ctrl)
        .flat_map(|row| row.keys)
        .find(|(key, _)| *key == code)
        .map(|(_, action)| action.clone())
}

/// The action `table`'s Ctrl rows bind to `key`, when Ctrl is held.
fn chord(table: &[Binding], key: KeyEvent) -> Option<Action> {
    let KeyCode::Char(ch) = key.code else {
        return None;
    };
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    let code = KeyCode::Char(ch.to_ascii_lowercase());
    table
        .iter()
        .filter(|row| row.ctrl)
        .flat_map(|row| row.keys)
        .find(|(key, _)| *key == code)
        .map(|(_, action)| action.clone())
}

/// A screen's own keys, then the global ones; a chord is no key there.
fn screen(table: &[Binding], key: KeyEvent) -> Option<Action> {
    if chorded(key) {
        return None;
    }
    lookup(table, key.code).or_else(|| global(key))
}

/// The movement keys, where a list takes them.
const MOVE: Binding = keys(
    &[
        (KeyCode::Up, Action::Move(Movement::Up)),
        (KeyCode::Down, Action::Move(Movement::Down)),
        (KeyCode::Char('k'), Action::Move(Movement::Up)),
        (KeyCode::Char('j'), Action::Move(Movement::Down)),
    ],
    "Move",
);

/// The keys that jump a page.
const JUMP: Binding = keys(
    &[
        (KeyCode::PageUp, Action::Move(Movement::PageUp)),
        (KeyCode::PageDown, Action::Move(Movement::PageDown)),
    ],
    "A page up or down",
);

/// The keys to the ends of a list.
const ENDS: Binding = keys(
    &[
        (KeyCode::Home, Action::Move(Movement::Home)),
        (KeyCode::End, Action::Move(Movement::End)),
        (KeyCode::Char('g'), Action::Move(Movement::Home)),
        (KeyCode::Char('G'), Action::Move(Movement::End)),
    ],
    "The first or last row",
);

/// The keys every screen takes (#1895), after its own: help, quit, the
/// log, writes, the command bar, profiles, connecting, and the screens.
pub(crate) const GLOBAL: &[Binding] = &[
    keys(
        &[(KeyCode::Char('?'), Action::ToggleHelp)],
        "This help; the movement keys scroll it",
    ),
    keys(
        &[(KeyCode::Char('q'), Action::Quit)],
        "Quit (asks first while writes are in effect)",
    ),
    Binding {
        keys: &[(KeyCode::Char('c'), Action::CtrlC)],
        ctrl: true,
        what: "Close a dialog or cancel the request; twice quits",
    },
    keys(
        &[(KeyCode::Char('L'), Action::ToggleLog)],
        "Show or hide the log pane",
    ),
    keys(
        &[
            (KeyCode::Char('W'), Action::ToggleArm),
            (KeyCode::Char('u'), Action::OpenLedger),
        ],
        "Arm or disarm writes (read-only until then); the ledger",
    ),
    keys(
        &[(KeyCode::Char(':'), Action::OpenCommandLine)],
        "The command bar, with the commands listed below",
    ),
    keys(
        &[
            (KeyCode::Char('p'), Action::OpenConnection),
            (KeyCode::Char('c'), Action::Reconnect),
        ],
        "The profile dialog; connect again, or retry",
    ),
    keys(
        &[
            (KeyCode::Char('1'), Action::ShowScreen(Screen::Main)),
            (KeyCode::Char('2'), Action::ShowScreen(Screen::Browse)),
        ],
        "Devices; 2 is Browse, the open device's browser",
    ),
    keys(
        &[
            (KeyCode::Char('3'), Action::ShowScreen(Screen::Watch)),
            (KeyCode::Char('4'), Action::ShowScreen(Screen::Network)),
            (KeyCode::Char('5'), Action::ShowScreen(Screen::Sc)),
            (KeyCode::Char('6'), Action::ShowScreen(Screen::Mstp)),
            (KeyCode::Char('7'), Action::ShowScreen(Screen::Capture)),
        ],
        "Watch, Network, SC, MS/TP, Capture; see the top line",
    ),
];

/// The device table (screen 1).
pub(crate) const DEVICES: &[Binding] = &[
    keys(
        &[(KeyCode::Char('d'), Action::OpenWhoIs)],
        "The Who-Is form",
    ),
    keys(
        &[(KeyCode::Enter, Action::Open)],
        "Browse the selected device (the one kept open comes back as it was)",
    ),
    MOVE,
    JUMP,
    ENDS,
    keys(
        &[
            (KeyCode::Char('/'), Action::StartFilter),
            (KeyCode::Char('s'), Action::CycleSort),
            (KeyCode::Char('S'), Action::ReverseSort),
        ],
        "Filter (Esc clears it); sort by the next column; reverse",
    ),
    keys(
        &[(KeyCode::Char('x'), Action::Disconnect)],
        "Disconnect, or cancel a connection attempt",
    ),
    keys(
        &[(KeyCode::Esc, Action::Escape)],
        "Clear the filter, or a notice",
    ),
];

/// The browser (screen 2): panes, reads, watching and writes.
pub(crate) const BROWSER: &[Binding] = &[
    keys(
        &[(KeyCode::Enter, Action::Open)],
        "Read the selected object",
    ),
    keys(
        &[
            (KeyCode::Tab, Action::CyclePane { back: false }),
            (KeyCode::BackTab, Action::CyclePane { back: true }),
        ],
        "Next or previous pane; below 120 columns a commandable object's priority array is one",
    ),
    keys(
        &[
            (KeyCode::Char('/'), Action::StartFilter),
            (KeyCode::Char('r'), Action::Refresh),
        ],
        "Filter the objects; read the object again",
    ),
    keys(
        &[(KeyCode::Char('a'), Action::AddWatch)],
        "Watch the selected object's present-value",
    ),
    keys(
        &[
            (KeyCode::Char('w'), Action::OpenWrite { relinquish: false }),
            (KeyCode::Char('n'), Action::OpenWrite { relinquish: true }),
        ],
        "In a write session: write a property; relinquish a slot",
    ),
    MOVE,
    JUMP,
    ENDS,
    keys(&[(KeyCode::Char('x'), Action::Disconnect)], "Disconnect"),
    keys(
        &[(KeyCode::Esc, Action::Escape)],
        "Clear the filter, then close the device and go back (1 keeps it)",
    ),
];

/// The watch list (screen 3): move, remove, back.
pub(crate) const WATCH: &[Binding] = &[
    MOVE,
    JUMP,
    ENDS,
    keys(
        &[
            (KeyCode::Char('x'), Action::RemoveWatch),
            (KeyCode::Delete, Action::RemoveWatch),
        ],
        "Stop watching the row and cancel its subscription",
    ),
    keys(
        &[(KeyCode::Esc, Action::ShowScreen(Screen::Main))],
        "Back to the devices",
    ),
];

/// The Network screen (screen 4): read, survey, switch tables.
pub(crate) const NETWORK: &[Binding] = &[
    keys(
        &[
            (
                KeyCode::Tab,
                Action::Network(NetworkKey::Cycle { back: false }),
            ),
            (
                KeyCode::BackTab,
                Action::Network(NetworkKey::Cycle { back: true }),
            ),
        ],
        "Next or previous table: BDT, FDT, Matrix, Networks; from 160 columns, the next pane",
    ),
    keys(
        &[(KeyCode::Char('r'), Action::Network(NetworkKey::Refresh))],
        "Read the BBMD's tables again; on Networks, ask the routers",
    ),
    keys(
        &[(KeyCode::Char('m'), Action::Network(NetworkKey::Survey))],
        "Survey every BBMD the tables lead to, for the matrix",
    ),
    keys(&[(KeyCode::Char('x'), Action::Disconnect)], "Disconnect"),
    keys(
        &[(KeyCode::Esc, Action::ShowScreen(Screen::Main))],
        "Back to the devices",
    ),
];

/// The SC screen (screen 5): switch views.
pub(crate) const SC: &[Binding] = &[
    keys(
        &[
            (KeyCode::Tab, Action::Sc(ScKey::Cycle { back: false })),
            (KeyCode::BackTab, Action::Sc(ScKey::Cycle { back: true })),
        ],
        "Next or previous view: hub certificates, doctor, my credentials; from 160 columns the credentials view shows the hub's chain on its left",
    ),
    keys(&[(KeyCode::Char('x'), Action::Disconnect)], "Disconnect"),
    keys(
        &[(KeyCode::Esc, Action::ShowScreen(Screen::Main))],
        "Back to the devices",
    ),
];

/// The MS/TP screen (screen 6): `z` counts from now, Space pauses or
/// follows; on the monitor, Tab changes view, `J` starts the join wizard,
/// `+` and `-` pick a MAC and Enter takes it to the join confirmation.
pub(crate) const MSTP: &[Binding] = &[
    keys(
        &[(KeyCode::Char('z'), Action::Mstp(MstpKey::Clear))],
        "Count from now",
    ),
    keys(
        &[(KeyCode::Char(' '), Action::Mstp(MstpKey::ToggleWatch))],
        "Pause the screen, or follow again",
    ),
    keys(
        &[(KeyCode::Char('x'), Action::Disconnect)],
        "Disconnect, or stop the monitor",
    ),
    keys(
        &[
            (KeyCode::Tab, Action::Mstp(MstpKey::Cycle { back: false })),
            (
                KeyCode::BackTab,
                Action::Mstp(MstpKey::Cycle { back: true }),
            ),
        ],
        "Monitor: stations, token ring, findings, errors, scan",
    ),
    keys(
        &[(KeyCode::Char('J'), Action::Mstp(MstpKey::Join))],
        "Monitor: start the safe join",
    ),
    keys(
        &[
            (KeyCode::Down, Action::Mstp(MstpKey::Pick { back: false })),
            (KeyCode::Up, Action::Mstp(MstpKey::Pick { back: true })),
            (
                KeyCode::Char('+'),
                Action::Mstp(MstpKey::Pick { back: false }),
            ),
            (
                KeyCode::Char('='),
                Action::Mstp(MstpKey::Pick { back: false }),
            ),
            (
                KeyCode::Char('-'),
                Action::Mstp(MstpKey::Pick { back: true }),
            ),
        ],
        "Safe join MAC, or Scan evidence row: next/previous",
    ),
    keys(
        &[(KeyCode::Enter, Action::Mstp(MstpKey::Accept))],
        "Safe join: review the proposal in the join dialog",
    ),
    keys(
        &[(KeyCode::Esc, Action::Mstp(MstpKey::Back))],
        "Back to the devices",
    ),
];

/// The Capture screen (screen 7): Tab between the list, the tree and the
/// hex view, Space to pause, `/` to filter, `e` to export, `z` to clear.
/// Left and Right work in the tree only ([`capture`]).
pub(crate) const CAPTURE: &[Binding] = &[
    keys(
        &[(KeyCode::Char(' '), Action::Capture(CaptureKey::Pause))],
        "Pause the list, or follow the newest frame",
    ),
    keys(
        &[
            (
                KeyCode::Tab,
                Action::Capture(CaptureKey::Cycle { back: false }),
            ),
            (
                KeyCode::BackTab,
                Action::Capture(CaptureKey::Cycle { back: true }),
            ),
        ],
        "List, decode tree, hex view",
    ),
    keys(
        &[
            (KeyCode::Enter, Action::Capture(CaptureKey::Toggle)),
            (KeyCode::Left, Action::Capture(CaptureKey::Collapse)),
            (KeyCode::Right, Action::Capture(CaptureKey::Expand)),
        ],
        "To the tree; close or open a node",
    ),
    keys(
        &[(KeyCode::Char('/'), Action::StartFilter)],
        "Filter: dev: mac: net: svc: pdu: errors bcast",
    ),
    keys(
        &[
            (
                KeyCode::Char('e'),
                Action::Capture(CaptureKey::Export { raw_ipv4: false }),
            ),
            (
                KeyCode::Char('E'),
                Action::Capture(CaptureKey::Export { raw_ipv4: true }),
            ),
        ],
        "Export to pcap: B/IP as Ethernet, or as raw IPv4",
    ),
    keys(
        &[(KeyCode::Char('z'), Action::Capture(CaptureKey::Clear))],
        "Forget every frame",
    ),
    keys(&[(KeyCode::Char('x'), Action::Disconnect)], "Disconnect"),
    MOVE,
    JUMP,
    ENDS,
    keys(
        &[(KeyCode::Esc, Action::Capture(CaptureKey::Back))],
        "Clear the filter, then back to the devices",
    ),
];

/// The global keys, when the screen's own don't take the key.
fn global(key: KeyEvent) -> Option<Action> {
    lookup(GLOBAL, key.code)
}

/// The help overlay scrolls with the movement keys.
fn help(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc | KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('q') => Action::Quit,
        code => Action::ScrollHelp(movement(code)?),
    })
}

/// The device table (screen 1).
fn main(key: KeyEvent) -> Option<Action> {
    screen(DEVICES, key)
}

/// The browser (screen 2).
fn browse(key: KeyEvent) -> Option<Action> {
    screen(BROWSER, key)
}

/// The watch list (screen 3).
fn watch(key: KeyEvent) -> Option<Action> {
    screen(WATCH, key)
}

/// The Network screen (screen 4).
fn network(key: KeyEvent) -> Option<Action> {
    screen(NETWORK, key)
}

/// The SC screen (screen 5).
fn sc(key: KeyEvent) -> Option<Action> {
    screen(SC, key)
}

/// The MS/TP screen (screen 6).
fn mstp(key: KeyEvent) -> Option<Action> {
    screen(MSTP, key)
}

/// The Capture screen (screen 7): Left and Right only in the decode tree.
fn capture(app: &App, key: KeyEvent) -> Option<Action> {
    let list = app.capture.focus == Focus::List;
    if list && matches!(key.code, KeyCode::Left | KeyCode::Right) {
        return None;
    }
    screen(CAPTURE, key)
}

/// Write session dialogs: forms, confirmations, the ledger and the exit
/// guard take typed characters; `q` types a `q` here rather than quitting.
fn safety(key: KeyEvent) -> Option<Action> {
    Some(Action::Modal(match key.code {
        KeyCode::Esc => ModalKey::Esc,
        KeyCode::Enter => ModalKey::Enter,
        KeyCode::Tab | KeyCode::Down => ModalKey::Next,
        KeyCode::BackTab | KeyCode::Up => ModalKey::Prev,
        KeyCode::Backspace => ModalKey::Backspace,
        _ => ModalKey::Char(typed(key)?),
    }))
}

/// The MS/TP join confirmation: `y` joins (T2); Up and Down pick a port.
fn join(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc | KeyCode::Char('n' | 'q') => Action::Escape,
        KeyCode::Up | KeyCode::Char('k') => Action::Join(JoinKey::Up),
        KeyCode::Down | KeyCode::Char('j') => Action::Join(JoinKey::Down),
        KeyCode::Char('y') => Action::Join(JoinKey::Confirm),
        _ => return None,
    })
}

fn command_line(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc => Action::CommandLine(FilterKey::Clear),
        KeyCode::Enter => Action::CommandLine(FilterKey::Commit),
        KeyCode::Backspace => Action::CommandLine(FilterKey::Backspace),
        _ => Action::CommandLine(FilterKey::Char(typed(key)?)),
    })
}

fn filter(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc => Action::Filter(FilterKey::Clear),
        KeyCode::Enter => Action::Filter(FilterKey::Commit),
        KeyCode::Backspace => Action::Filter(FilterKey::Backspace),
        KeyCode::Up => Action::Move(Movement::Up),
        KeyCode::Down => Action::Move(Movement::Down),
        _ => Action::Filter(FilterKey::Char(typed(key)?)),
    })
}

fn form(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc => Action::Escape,
        KeyCode::Enter => Action::Form(FormKey::Submit),
        KeyCode::Tab | KeyCode::Down => Action::Form(FormKey::Next),
        KeyCode::BackTab | KeyCode::Up => Action::Form(FormKey::Prev),
        KeyCode::Left => Action::Form(FormKey::Left),
        KeyCode::Right => Action::Form(FormKey::Right),
        KeyCode::Backspace => Action::Form(FormKey::Backspace),
        _ => Action::Form(FormKey::Char(typed(key)?)),
    })
}

fn picker(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Action::Quit,
        KeyCode::Up | KeyCode::Char('k') => Action::Picker(PickerKey::Up),
        KeyCode::Down | KeyCode::Char('j') => Action::Picker(PickerKey::Down),
        KeyCode::Enter => Action::Picker(PickerKey::Choose),
        KeyCode::Char(ch @ '1'..='9') => {
            Action::Picker(PickerKey::Number(ch as usize - '0' as usize))
        }
        _ => return None,
    })
}

fn connection(app: &App, key: KeyEvent) -> Option<Action> {
    use super::app::connection::{Key, Screen};
    let connection = app.connection.as_ref()?;
    let editing = connection.screen == Screen::Edit;
    if key.code == KeyCode::Esc {
        return Some(if editing {
            Action::Connection(Key::Discard)
        } else {
            Action::Escape
        });
    }
    if !editing && key.code == KeyCode::Char('q') {
        return Some(Action::Quit);
    }
    // Disconnect from the connection screen too; a no-op when nothing is up.
    if !editing && key.code == KeyCode::Char('x') {
        return Some(Action::Disconnect);
    }
    Some(Action::Connection(match key.code {
        KeyCode::Tab | KeyCode::Down => Key::Next,
        KeyCode::BackTab | KeyCode::Up => Key::Prev,
        KeyCode::Enter => Key::Submit,
        KeyCode::Backspace if editing => Key::Backspace,
        _ if editing => Key::Char(typed(key)?),
        // As in the other lists (#1894).
        KeyCode::Char('j') => Key::Next,
        KeyCode::Char('k') => Key::Prev,
        KeyCode::Char('n') => Key::New,
        KeyCode::Char('s') => Key::NewSc,
        KeyCode::Char('m') => Key::NewMstp,
        KeyCode::Char('e') => Key::Edit,
        KeyCode::Char('r') => Key::Reload,
        KeyCode::Char('p') => Key::List,
        KeyCode::Char('c') => Key::Connect,
        KeyCode::Char('a') => Key::Attest,
        KeyCode::Char('y') => Key::Confirm,
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No key is bound twice on a screen: the table's own keys and the
    /// global ones are apart, so the order of the rows never decides.
    #[test]
    fn no_screen_binds_a_key_twice() {
        for (name, table) in [
            ("devices", DEVICES),
            ("browser", BROWSER),
            ("watch", WATCH),
            ("network", NETWORK),
            ("sc", SC),
            ("mstp", MSTP),
            ("capture", CAPTURE),
        ] {
            let mut seen = Vec::new();
            for row in table.iter().chain(GLOBAL).filter(|row| !row.ctrl) {
                for (code, _) in row.keys {
                    assert!(!seen.contains(code), "{name} binds {code:?} twice");
                    seen.push(*code);
                }
            }
        }
    }

    #[test]
    fn rows_are_labelled_as_help_names_the_keys() {
        assert_eq!(GLOBAL[2].label(), "Ctrl-C");
        assert_eq!(MOVE.label(), "Up Down k j");
        assert_eq!(JUMP.label(), "PgUp PgDn");
        assert_eq!(CAPTURE[0].label(), "Space");
        assert_eq!(CAPTURE[1].label(), "Tab Shift-Tab");
    }
}
