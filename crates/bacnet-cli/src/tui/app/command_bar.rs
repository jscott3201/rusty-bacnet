//! The `:` command bar. It reads the one-shot syntax for the commands the
//! TUI runs itself:
//!
//! - `watch DEVICE OBJECT [PROPERTY]` (or `w`): add a watch row. DEVICE is
//!   a device from the device table, in the shared target grammar: its
//!   instance, `dnet:instance` for a routed one, or the routed station the
//!   table shows, `dnet:0xMAC`; OBJECT is `ai:1`, `analog-input:1` or
//!   `analog-input,1`; PROPERTY defaults to present-value.
//! - `csv on|off`: log watched values to the profile workspace, or stop.
//! - `writem DEVICE OBJECT PROPERTY VALUE[@PRIORITY] ...`: one
//!   WritePropertyMultiple to the device, any number of `OBJECT PROPERTY
//!   VALUE` groups, previewed and confirmed at T2 or above. DEVICE is a row
//!   of the device table as `watch` takes it, or its address; the write
//!   goes to that row's address. Needs an armed write session.
//! - `bbmd [ADDRESS]`: show and read that BBMD's tables on the Network screen
//!   (`ip` or `ip:port`, port 47808 by default); `bbmd` alone goes back to
//!   the tables of the BBMD the session registers with.
//! - `find NAME`: in active mode broadcast a Who-Has on this network
//!   alone, or with `--dnet N` first on one remote network and with
//!   `--global` on every network (#2596); in passive mode search local
//!   observed names. NAME is the rest of the line as typed after any flag
//!   (one quoted word is unwrapped, so a name that starts `--` is quoted),
//!   and list the answers below the device table; `find` alone, or
//!   `find ""`, closes the list.
//! - `mstp rotation MS`, `mstp hold MS`, `mstp listen SECS`: the MS/TP
//!   monitor's thresholds, and how long its join wizard listens.
//! - `dcc ACTION [MINUTES]` (or `control`) and `reinit STATE`:
//!   DeviceCommunicationControl and ReinitializeDevice to the selected
//!   device, the action and state named as the one-shot `control` and
//!   `reinit` name them. A masked password prompt and the T3 confirmation
//!   follow; both need an armed write session on a profile with
//!   `dangerous = true`.
//!
//! Every command but `find` is split into words by [`words::split`]: quotes
//! group words and a backslash escapes a quote, a whitespace character or a
//! backslash. A quoted or escaped VALUE in `writem` is a CharacterString.

mod words;

use std::time::Duration;

use bacnet_ops::parse::{parse_object_specifier, parse_property, parse_value_for_with_priority};
use bacnet_ops::target::{Target, parse_target};
use bacnet_ops::{DeviceAddress, Priority, Scope};
use bacnet_types::enums::{EnableDisable, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use self::words::Word;
use super::devices::DeviceTable;
use super::monitor::Setting;
use super::{App, FLASH_FOR, FilterKey, Overlay};
use crate::commands::device::{parse_enable_disable, parse_reinit_state};
use crate::tui::message::{Command, DeviceRow};
use crate::tui::safety::WriteSpec;
use crate::tui::safety::manage::{Action, DccTime};
use crate::tui::watched::{Point, WatchCommand};

/// One command: its name, a short alias, its forms and what it does. The
/// parser finds the command here, and the command bar's hint, the help
/// overlay and the usage line all read the same rows (#1895), so none can
/// name a command the parser doesn't take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Syntax {
    /// The first word.
    pub(crate) name: &'static str,
    /// Another first word for it, if any.
    pub(crate) alias: Option<&'static str>,
    /// Each form, as typed.
    pub(crate) usage: &'static [&'static str],
    /// What it does, in a few words.
    pub(crate) what: &'static str,
}

impl Syntax {
    /// What `form`, one of the command's forms, does under the session
    /// policy: `find`'s scoped forms send further than this network, and
    /// a passive session's `find` searches what it overheard.
    pub(crate) fn description(&self, form: &str, passive: bool) -> &'static str {
        match (self.name, form) {
            ("find", "find NAME") if passive => {
                "Search local observed object names; alone, close the list"
            }
            ("find", _) if passive => "Not in a passive session, which sends no Who-Has",
            ("find", "find --dnet N NAME") => "Who-Has by object name on remote network N",
            ("find", "find --global NAME") => "Who-Has by object name on every network",
            _ => self.what,
        }
    }
}

/// Every command the bar takes.
pub(crate) const COMMANDS: &[Syntax] = &[
    Syntax {
        name: "watch",
        alias: Some("w"),
        usage: &["watch DEVICE OBJECT [PROPERTY]"],
        what: "Watch a property (present-value by default); w for short",
    },
    Syntax {
        name: "csv",
        alias: None,
        usage: &["csv on|off"],
        what: "Log the watched values to the workspace, or stop",
    },
    Syntax {
        name: "writem",
        alias: None,
        usage: &["writem DEVICE OBJECT PROPERTY VALUE[@P] ..."],
        what: "One WritePropertyMultiple to a device, previewed first",
    },
    Syntax {
        name: "bbmd",
        alias: None,
        usage: &["bbmd [ADDRESS]"],
        what: "Another BBMD's tables; alone, back to our own BBMD's",
    },
    Syntax {
        name: "find",
        alias: None,
        usage: &["find NAME", "find --dnet N NAME", "find --global NAME"],
        what: "Who-Has by object name; alone, close the list",
    },
    Syntax {
        name: "mstp",
        alias: None,
        usage: &[
            "mstp rotation|hold MS",
            "mstp listen SECS",
            "mstp scan SECS|cancel",
        ],
        what: "Monitor thresholds, safe-join window, or receive-only baud scan",
    },
    Syntax {
        name: "dcc",
        alias: Some("control"),
        usage: &[
            "dcc enable",
            "dcc disable-initiation|disable [MINUTES]",
            "dcc disable-initiation|disable forever",
        ],
        what: "DeviceCommunicationControl, selected device (T3)",
    },
    Syntax {
        name: "reinit",
        alias: None,
        usage: &["reinit coldstart|warmstart|STATE"],
        what: "ReinitializeDevice, selected device (T3)",
    },
];

/// The command whose name or alias is `word`.
fn command(word: &str) -> Option<&'static Syntax> {
    COMMANDS
        .iter()
        .find(|syntax| syntax.name == word || syntax.alias == Some(word))
}

/// The usage line shown for an unknown command.
pub(crate) fn usage() -> String {
    let forms: Vec<&str> = COMMANDS
        .iter()
        .flat_map(|s| s.usage.iter().copied())
        .collect();
    format!("commands: {}", forms.join(" | "))
}

/// The commands that match what is typed so far, for the hint beside the
/// bar: the one named by a complete first word, or every one whose name
/// starts with a partial one (all of them before anything is typed).
pub(crate) fn matching(text: &str) -> Vec<&'static Syntax> {
    let first = text.split_whitespace().next().unwrap_or_default();
    let typed_on = text.trim_start().len() > first.len();
    if let Some(syntax) = command(first).filter(|_| typed_on || first.len() > 1) {
        return vec![syntax];
    }
    if typed_on {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter(|syntax| syntax.name.starts_with(first))
        .collect()
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    /// Watch a point.
    Watch(Point),
    /// Start or stop CSV logging.
    Csv(bool),
    /// A WritePropertyMultiple batch.
    Writem(Vec<WriteSpec>),
    /// Show a BBMD's tables.
    Bbmd(Option<std::net::SocketAddrV4>),
    /// Search for an object by name, where the scope says; an empty name
    /// closes the search.
    Find(String, Scope),
    /// An MS/TP monitor setting.
    Mstp(Setting),
    /// Start a baud scan, or cancel when no window is supplied.
    Scan(Option<Duration>),
    /// DeviceCommunicationControl or ReinitializeDevice to the selected
    /// device.
    Manage(Action),
}

/// `dcc ACTION [MINUTES|forever]`. A disable without a duration lasts
/// [`DEFAULT_DCC_MINUTES`](crate::tui::safety::manage::DEFAULT_DCC_MINUTES);
/// `forever` sends none. `enable` takes none,
/// since the device ignores one.
fn dcc(action: &str, duration: Option<&str>) -> Result<Action, String> {
    let state = parse_enable_disable(action)?;
    let time = match duration {
        _ if state == EnableDisable::ENABLE => match duration {
            None => DccTime::Forever,
            Some(_) => return Err("usage: dcc enable (it takes no duration)".into()),
        },
        None => DccTime::Default,
        Some("forever") => DccTime::Forever,
        Some(word) => DccTime::Minutes(
            word.parse::<u16>()
                .ok()
                .filter(|minutes| *minutes > 0)
                .ok_or_else(|| {
                    format!("'{word}': dcc takes 1 to 65535 whole minutes, or forever")
                })?,
        ),
    };
    Ok(Action::Dcc { state, time })
}

/// `mstp rotation|hold MS` or `mstp listen SECS`, each at least 1.
fn mstp_setting(which: &str, value: &str) -> Result<Setting, String> {
    let number = value
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=3_600_000).contains(n))
        .ok_or_else(|| format!("'{value}': a whole number from 1"))?;
    match which {
        "rotation" => Ok(Setting::Rotation(Duration::from_millis(number))),
        "hold" => Ok(Setting::Hold(Duration::from_millis(number))),
        "listen" if number <= 3_600 => Ok(Setting::Listen(Duration::from_secs(number))),
        "listen" => Err("mstp listen takes 1 to 3600 seconds".into()),
        _ => Err(format!(
            "'{which}': mstp rotation|hold MS, or mstp listen SECS"
        )),
    }
}

/// A device instance, with the network when typed as `dnet:instance`, or
/// the device the table lists at a routed station typed as `dnet:0xMAC`.
fn device(word: &str, command: &str, devices: &DeviceTable) -> Result<(u32, Option<u16>), String> {
    match parse_target(word)? {
        Target::Instance(instance) => Ok((instance, None)),
        Target::Routed { dnet, instance } => Ok((instance, Some(dnet))),
        Target::RoutedMac { dnet, mac } => {
            let row = at_station(devices, word, dnet, &mac)?;
            Ok((row.instance, Some(dnet)))
        }
        _ => Err(format!(
            "'{word}': {command} takes a device from the device table \
             (N, dnet:N or dnet:0xMAC)"
        )),
    }
}

/// The row of the device table `word` names, for a command that goes to
/// one address (#2560): its instance (`dnet:N` on one network), the routed
/// station the table shows (`dnet:0xMAC`), or a local address it lists.
fn row<'a>(word: &str, command: &str, devices: &'a DeviceTable) -> Result<&'a DeviceRow, String> {
    let missing =
        || format!("'{word}': no device in the device table is that one; run a Who-Is first");
    match parse_target(word)? {
        Target::Instance(instance) => {
            one_claim(word, devices.claims(instance)).ok_or_else(missing)?
        }
        // Scoped to its network: a claim elsewhere doesn't make it ambiguous.
        Target::Routed { dnet, instance } => {
            let claims = devices.claims(instance);
            let here = claims.into_iter().filter(|row| row.network == Some(dnet));
            one_claim(word, here.collect()).ok_or_else(missing)?
        }
        Target::RoutedMac { dnet, mac } => at_station(devices, word, dnet, &mac),
        Target::Mac(mac) => {
            let at = DeviceAddress::local(&mac).map_err(|why| format!("'{word}': {why}"))?;
            devices.at(&at).ok_or_else(missing)
        }
        Target::Bip6(_) => Err(format!(
            "'{word}': {command} takes a device from the device table \
             (N, dnet:N, dnet:0xMAC or its address)"
        )),
    }
}

/// The one row of `claims`, the rows claiming the instance `word` names:
/// `None` when there are none, and an error naming each address when two
/// or more do, so the operator can name one by it.
fn one_claim<'a>(word: &str, claims: Vec<&'a DeviceRow>) -> Option<Result<&'a DeviceRow, String>> {
    match claims.as_slice() {
        [] => None,
        [row] => Some(Ok(row)),
        several => {
            let at: Vec<&str> = several.iter().map(|row| row.address.as_str()).collect();
            Some(Err(format!(
                "'{word}': {} devices in the device table claim that instance ({}); name one by \
                 its address",
                several.len(),
                at.join(", ")
            )))
        }
    }
}

/// The device the table lists at station `mac` on network `dnet` (#1914):
/// the table holds one row per address (#2560).
fn at_station<'a>(
    devices: &'a DeviceTable,
    word: &str,
    dnet: u16,
    mac: &[u8],
) -> Result<&'a DeviceRow, String> {
    DeviceAddress::new(dnet, mac)
        .ok()
        .and_then(|at| devices.at(&at))
        .ok_or_else(|| {
            format!(
                "'{word}': no device in the device table is at that station; run a Who-Is first"
            )
        })
}

fn object(word: &str) -> Result<ObjectIdentifier, String> {
    let (object_type, instance) = parse_object_specifier(word)?;
    ObjectIdentifier::new(object_type, instance).map_err(|e| format!("'{word}': {e}"))
}

/// A `VALUE[@P]` word as the value grammar's text and the priority. A word
/// with a quoted or escaped part is a CharacterString, its priority an `@P`
/// after the last such part; any other word is read by the one-shot value
/// grammar for the property written, which knows its names.
fn value(
    word: &Word,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<(String, Option<u8>), String> {
    let Some(literal_to) = word.literal_to else {
        let (_, priority) =
            parse_value_for_with_priority(object.object_type(), property, index, &word.text)?;
        let text = match priority {
            Some(_) => word.text.rsplit_once('@').map_or(&*word.text, |(v, _)| v),
            None => &word.text,
        };
        return Ok((text.to_string(), priority.map(Priority::get)));
    };
    let (text, priority) = match word.text[literal_to..].find('@') {
        Some(at) => {
            let (text, priority) = word.text.split_at(literal_to + at);
            let priority = &priority[1..];
            match priority.parse::<Priority>() {
                Ok(p) => (text, Some(p.get())),
                Err(_) => return Err(format!("invalid priority '{priority}': 1-16")),
            }
        }
        None => (&*word.text, None),
    };
    // The value grammar's string form: its outer quotes are taken off whole.
    Ok((format!("\"{text}\""), priority))
}

/// `find`'s line after the command: `--dnet N` or `--global` first, for
/// where the Who-Has goes (this network alone without either), then the
/// name ([`find_name`]).
fn find_line(rest: &str) -> Result<Line, String> {
    const USAGE: &str = "usage: find NAME | find --dnet N NAME | find --global NAME";
    let word = |text: &str| -> (String, String) {
        let (first, after) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        (first.to_string(), after.trim_start().to_string())
    };
    let (first, after) = word(rest);
    let (scope, name) = match first.as_str() {
        "--global" => (Scope::Global, after),
        "--dnet" => {
            let (network, after) = word(&after);
            let network = crate::core::range::parse_dnet(&network)?;
            (Scope::Network(network), after)
        }
        // Another flag is a typo, not a name: a name that starts `--` is
        // quoted.
        flag if flag.starts_with("--") => return Err(USAGE.into()),
        _ => return Ok(Line::Find(find_name(rest), Scope::Local)),
    };
    // One flag, then a name: a second flag, or no name, is no search.
    let name = (!name.starts_with("--"))
        .then(|| find_name(&name))
        .filter(|name| !name.is_empty())
        .ok_or(USAGE)?;
    Ok(Line::Find(name, scope))
}

/// The name `find` looks for: the rest of the line as typed, or, when all
/// of it is one quoted word, that word unwrapped (`"Zone  Temp"`). An empty
/// name closes the list.
fn find_name(rest: &str) -> String {
    let quoted = matches!(rest.chars().next(), Some(q @ ('"' | '\''))
        if rest.len() >= 2 && rest.ends_with(q));
    if quoted && let Ok([word]) = words::split(rest).as_deref() {
        return word.text.clone();
    }
    rest.to_string()
}

/// `OBJECT PROPERTY VALUE[@P]` groups to the device of `row`, checked with
/// the one-shot parsers.
fn writes(row: &DeviceRow, words: &[Word]) -> Result<Vec<WriteSpec>, String> {
    if words.is_empty() || !words.len().is_multiple_of(3) {
        return Err("writem DEVICE then OBJECT PROPERTY VALUE[@P] groups".into());
    }
    words
        .chunks(3)
        .map(|group| {
            let (property, index) = parse_property(&group[1].text)?;
            let object = object(&group[0].text)?;
            let (value, priority) = value(&group[2], object, property, index)?;
            Ok(WriteSpec {
                address: row.at.clone(),
                device: row.instance,
                object,
                property,
                index,
                value: Some(value),
                priority,
            })
        })
        .collect()
}

/// Parse one command line: its first word picks the row of [`COMMANDS`],
/// and the rest is read by that row's form. A routed station is looked up
/// in `devices`.
pub(crate) fn parse(text: &str, devices: &DeviceTable) -> Result<Line, String> {
    // `find` reads the rest of the line itself, before any splitting, so a
    // name such as `Joe's office` needs no quotes.
    let line = text.trim();
    let (head, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    if command(head).is_some_and(|syntax| syntax.name == "find") {
        return find_line(rest.trim());
    }
    let all = words::split(text)?;
    let Some((first, args)) = all.split_first() else {
        return Err(usage());
    };
    let Some(syntax) = command(&first.text) else {
        return Err(format!("unknown command '{}'; {}", text.trim(), usage()));
    };
    let wrong = || format!("usage: {}", syntax.usage.join(" | "));
    let plain: Vec<&str> = args.iter().map(|w| w.text.as_str()).collect();
    match (syntax.name, plain.as_slice()) {
        ("writem", [target, ..]) => Ok(Line::Writem(writes(
            row(target, "writem", devices)?,
            &args[1..],
        )?)),
        ("watch", [target, object_word, rest @ ..]) => {
            let (device, network) = device(target, "watch", devices)?;
            let object = object(object_word)?;
            let property = match rest {
                [] => PropertyIdentifier::PRESENT_VALUE,
                [property] => match parse_property(property)? {
                    (property, None) => property,
                    (_, Some(_)) => return Err("the watch list takes no array index".into()),
                },
                _ => return Err(format!("too many words; {}", wrong())),
            };
            Ok(Line::Watch(Point {
                device,
                network,
                object,
                property,
            }))
        }
        ("csv", ["on"]) => Ok(Line::Csv(true)),
        ("csv", ["off"]) => Ok(Line::Csv(false)),
        ("bbmd", []) => Ok(Line::Bbmd(None)),
        ("bbmd", [address]) => Ok(Line::Bbmd(Some(crate::foreign::parse_bbmd_arg(address)?))),
        ("mstp", ["scan", "cancel"]) => Ok(Line::Scan(None)),
        ("mstp", ["scan", value]) => {
            let seconds = value
                .parse::<u64>()
                .ok()
                .filter(|n| (1..=3600).contains(n))
                .ok_or_else(|| "mstp scan takes 1 to 3600 whole seconds, or cancel".to_string())?;
            Ok(Line::Scan(Some(Duration::from_secs(seconds))))
        }
        ("mstp", [which, value]) => Ok(Line::Mstp(mstp_setting(which, value)?)),
        ("dcc", [action, rest @ ..]) if rest.len() <= 1 => {
            Ok(Line::Manage(dcc(action, rest.first().copied())?))
        }
        ("reinit", [state]) => Ok(Line::Manage(Action::Reinit {
            state: parse_reinit_state(state)?,
        })),
        // A quoted first word, `"find"`, is still find.
        ("find", _) => find_line(rest.trim()),
        _ => Err(wrong()),
    }
}

impl App {
    /// `:`: open the command bar.
    pub(super) fn open_command_line(&mut self) {
        self.overlay = Overlay::Command(String::new());
    }

    /// A key on the command bar.
    pub(super) fn command_line_key(&mut self, key: FilterKey, out: &mut Vec<Command>) {
        let Overlay::Command(text) = &mut self.overlay else {
            return;
        };
        match key {
            FilterKey::Char(ch) => text.push(ch),
            FilterKey::Backspace => {
                text.pop();
            }
            FilterKey::Clear => self.overlay = Overlay::None,
            FilterKey::Commit => {
                let line = std::mem::take(text);
                self.overlay = Overlay::None;
                match parse(&line, &self.devices) {
                    Ok(Line::Watch(point)) => self.watch_point(point, out),
                    Ok(Line::Csv(on)) => out.push(Command::Watch(WatchCommand::Csv(on))),
                    Ok(Line::Writem(specs)) => self.prepare(specs, true, out),
                    Ok(Line::Bbmd(bbmd)) => self.show_bbmd(bbmd, out),
                    Ok(Line::Find(name, scope)) => self.find(name, scope, out),
                    Ok(Line::Mstp(setting)) => self.mstp_setting(setting),
                    Ok(Line::Scan(window)) => self.mstp_scan(window, out),
                    Ok(Line::Manage(action)) => self.open_manage(action),
                    Err(error) => self.flash(error, FLASH_FOR),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "command_bar_tests.rs"]
mod tests;
