//! The `:` command bar. It reads the one-shot syntax for the commands the
//! TUI runs itself:
//!
//! - `watch DEVICE OBJECT [PROPERTY]` (or `w`): add a watch row. DEVICE is
//!   a device from the device table, in the shared target grammar: its
//!   instance, `dnet:instance` for a routed one, or the routed station the
//!   table shows, `dnet:0xMAC`; OBJECT is `ai:1` or `analog-input:1`;
//!   PROPERTY defaults to present-value.
//! - `csv on|off`: log watched values to the profile workspace, or stop.
//! - `writem DEVICE OBJECT PROPERTY VALUE[@PRIORITY] ...`: one
//!   WritePropertyMultiple to the device, any number of `OBJECT PROPERTY
//!   VALUE` groups, previewed and confirmed at T2 or above. Needs an armed
//!   write session.
//! - `bbmd [ADDRESS]`: show and read that BBMD's tables on the Network screen
//!   (`ip` or `ip:port`, port 47808 by default); `bbmd` alone goes back to
//!   the tables of the BBMD the session registers with.
//! - `find NAME`: in active mode broadcast Who-Has; in passive mode search
//!   local observed names. NAME is the rest
//!   of the line as typed (one quoted word is unwrapped), and list the
//!   answers below the device table; `find` alone, or `find ""`, closes the
//!   list.
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
use bacnet_types::enums::{EnableDisable, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use self::words::Word;
use super::devices::DeviceTable;
use super::monitor::Setting;
use super::{App, FLASH_FOR, FilterKey, Overlay};
use crate::commands::device::{parse_enable_disable, parse_reinit_state};
use crate::tui::message::Command;
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
    /// The command description for the session policy.
    pub(crate) fn description(&self, passive: bool) -> &'static str {
        if passive && self.name == "find" {
            "Search local observed object names; alone, close the list"
        } else {
            self.what
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
        usage: &["find NAME"],
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
    /// Search for an object by name; empty closes the search.
    Find(String),
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
        Target::RoutedMac { dnet, mac } => Ok((at_station(devices, word, dnet, &mac)?, Some(dnet))),
        _ => Err(format!(
            "'{word}': {command} takes a device from the device table \
             (N, dnet:N or dnet:0xMAC)"
        )),
    }
}

/// The device the table lists at station `mac` on network `dnet` (#1914).
/// The table writes a routed device's address as that station, so it is
/// read back and compared as a target, whatever case the hex was typed in.
fn at_station(devices: &DeviceTable, word: &str, dnet: u16, mac: &[u8]) -> Result<u32, String> {
    let mut found: Vec<u32> = devices
        .rows()
        .filter(|row| row.network == Some(dnet))
        .filter(|row| {
            matches!(parse_target(&row.address),
                Ok(Target::RoutedMac { dnet: network, mac: station })
                    if network == dnet && station == mac)
        })
        .map(|row| row.instance)
        .collect();
    found.sort_unstable();
    match found.as_slice() {
        [instance] => Ok(*instance),
        [] => Err(format!(
            "'{word}': no device in the device table is at that station; run a Who-Is first"
        )),
        several => {
            let list: Vec<String> = several.iter().map(u32::to_string).collect();
            Err(format!(
                "'{word}': devices {} share that station; name one by its instance",
                list.join(", ")
            ))
        }
    }
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
        return Ok((text.to_string(), priority));
    };
    let (text, priority) = match word.text[literal_to..].find('@') {
        Some(at) => {
            let (text, priority) = word.text.split_at(literal_to + at);
            let priority = &priority[1..];
            match priority.parse::<u8>() {
                Ok(p) if (1..=16).contains(&p) => (text, Some(p)),
                _ => return Err(format!("invalid priority '{priority}': 1-16")),
            }
        }
        None => (&*word.text, None),
    };
    // The value grammar's string form: its outer quotes are taken off whole.
    Ok((format!("\"{text}\""), priority))
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

/// `OBJECT PROPERTY VALUE[@P]` groups, checked with the one-shot parsers.
fn writes(device: u32, words: &[Word]) -> Result<Vec<WriteSpec>, String> {
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
                device,
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
        return Ok(Line::Find(find_name(rest.trim())));
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
            device(target, "writem", devices)?.0,
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
        ("find", _) => Ok(Line::Find(find_name(rest.trim()))),
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
                    Ok(Line::Find(name)) => self.find(name, out),
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
mod tests {
    use bacnet_ops::target::routed_station;
    use bacnet_types::enums::{ObjectType, Segmentation};

    use super::*;
    use crate::tui::message::DeviceRow;

    /// A line read with an empty device table.
    fn parse(text: &str) -> Result<Line, String> {
        super::parse(text, &DeviceTable::default())
    }

    /// A table row for `instance`, routed at `station` when given.
    fn device_row(instance: u32, station: Option<(u16, &[u8])>) -> DeviceRow {
        DeviceRow {
            instance,
            address: match station {
                Some((network, mac)) => routed_station(network, mac).unwrap(),
                None => format!("10.0.0.{}:47808", instance % 250),
            },
            via: station.map(|_| "10.0.1.1:47808".into()),
            network: station.map(|(network, _)| network),
            vendor_id: 260,
            max_apdu: 480,
            segmentation: Segmentation::NONE,
            last_seen: std::time::Instant::now(),
        }
    }

    /// A routed station names the device the table lists there, in any
    /// case of hex; one not in the table, or on another network, is an
    /// error that says so (#1914).
    #[test]
    fn a_routed_station_names_the_device_the_table_lists_there() {
        let mut devices = DeviceTable::default();
        devices.upsert(device_row(2001, Some((2, &[5]))));
        devices.upsert(device_row(2002, Some((2, &[0x7F, 0, 0, 1, 0xBA, 0xC0]))));
        devices.upsert(device_row(100, None));
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let watch = |device, network| {
            Ok(Line::Watch(Point {
                device,
                network: Some(network),
                object: ai1,
                property: PropertyIdentifier::PRESENT_VALUE,
            }))
        };
        assert_eq!(super::parse("watch 2:0x05 ai:1", &devices), watch(2001, 2));
        assert_eq!(
            super::parse("w 2:0x7f000001bac0 ai:1", &devices),
            watch(2002, 2)
        );
        let Ok(Line::Writem(specs)) = super::parse("writem 2:0x05 av:1 pv 4", &devices) else {
            panic!("expected writem");
        };
        assert_eq!(specs[0].device, 2001);
        // A MAC is matched octet for octet: 2:0x0005 is two octets, not
        // station 5.
        for missing in [
            "watch 2:0x06 ai:1",
            "watch 3:0x05 ai:1",
            "watch 2:0x0005 ai:1",
            "writem 2:0x06 av:1 pv 4",
        ] {
            let error = super::parse(missing, &devices).unwrap_err();
            assert!(error.contains("no device in the device table"), "{error}");
        }
        // Two devices behind one station: name one by instance.
        devices.upsert(device_row(2003, Some((2, &[5]))));
        let error = super::parse("watch 2:0x05 ai:1", &devices).unwrap_err();
        assert!(error.contains("devices 2001, 2003 share"), "{error}");
    }

    /// `find` takes the rest of the line as typed, apostrophes and runs of
    /// spaces included; only a name that is one quoted word is unwrapped,
    /// and `find ""` closes the list like a bare `find`.
    #[test]
    fn find_takes_the_rest_of_the_line_as_typed() {
        let find = |line: &str| match parse(line) {
            Ok(Line::Find(name)) => name,
            other => panic!("{line}: {other:?}"),
        };
        assert_eq!(find("find Joe's office"), "Joe's office");
        assert_eq!(find("find  Supply  Air Temp "), "Supply  Air Temp");
        assert_eq!(find(r#"find "Zone  Temp""#), "Zone  Temp");
        assert_eq!(find(r#"find 'Zone "A"'"#), r#"Zone "A""#);
        assert_eq!(find(r#"find "a\"b""#), r#"a"b"#);
        assert_eq!(find(r#"find """#), "");
        assert_eq!(find("find"), "");
        // More than one quoted word, or an unbalanced one, stays as typed.
        assert_eq!(find(r#"find "a" "b""#), r#""a" "b""#);
        assert_eq!(find(r#"find "Zone 1"#), r#""Zone 1"#);
        assert_eq!(find(r"find C:\temp"), r"C:\temp");
        assert_eq!(find("\"find\" x  y"), "x  y");
    }

    /// A quoted VALUE is a CharacterString, spaces and all, with any
    /// priority after the quote; a quote left open is refused (#1800).
    #[test]
    fn writem_takes_quoted_values() {
        let values = |line: &str| -> Vec<(Option<String>, Option<u8>)> {
            match parse(line) {
                Ok(Line::Writem(specs)) => {
                    specs.into_iter().map(|s| (s.value, s.priority)).collect()
                }
                other => panic!("{line}: {other:?}"),
            }
        };
        let string = |text: &str| Some(format!("\"{text}\""));
        assert_eq!(
            values(r#"writem 100 av:1 description "two words""#),
            [(string("two words"), None)]
        );
        assert_eq!(
            values(r#"writem 100 av:1 description 'two  words' av:2 pv 5@9"#),
            [(string("two  words"), None), (Some("5".into()), Some(9))]
        );
        // The priority goes after the quote; an @ inside it is text.
        assert_eq!(
            values(r#"writem 100 av:1 description "a@b"@8"#),
            [(string("a@b"), Some(8))]
        );
        assert_eq!(
            values(r#"writem 100 av:1 description """#),
            [(string(""), None)]
        );
        assert_eq!(
            values(r#"writem 100 av:1 description "say \"hi\"""#),
            [(string(r#"say "hi""#), None)]
        );
        // A quoted part anywhere makes the word a string.
        assert_eq!(
            values(r#"writem 100 av:1 description zone' 'four"#),
            [(string("zone four"), None)]
        );
        // An escape makes the word a string too, escaped quotes and all,
        // with the priority after the last escape.
        assert_eq!(
            values(r#"writem 100 av:1 description \"x\""#),
            [(string(r#""x""#), None)]
        );
        assert_eq!(
            values(r"writem 100 av:1 description Zone\ 2@9 av:2 pv 4"),
            [(string("Zone 2"), Some(9)), (Some("4".into()), None)]
        );
        // A quoted number is a string, which a numeric property refuses
        // when the write is planned.
        assert_eq!(
            values("writem 100 av:1 pv '72.5'"),
            [(string("72.5"), None)]
        );
        assert!(parse(r"writem 100 av:1 description Zone\ 2@17").is_err());
        // A backslash that escapes nothing is plain text, not a string.
        assert!(
            parse(r"writem 100 av:1 description C:\temp")
                .unwrap_err()
                .contains("cannot parse value")
        );
        // Each value reads back as the string typed.
        let Some(text) = string("two words") else {
            unreachable!()
        };
        assert_eq!(
            bacnet_ops::parse::parse_value(&text),
            Ok(bacnet_types::primitives::PropertyValue::CharacterString(
                "two words".into()
            ))
        );
        assert!(
            parse(r#"writem 100 av:1 description "x"@17"#)
                .unwrap_err()
                .contains("priority")
        );
        assert!(parse(r#"writem 100 av:1 description "x"@"#).is_err());
        assert!(
            parse(r#"writem 100 av:1 description "x"@0"#)
                .unwrap_err()
                .contains("priority")
        );
        let open = parse(r#"writem 100 av:1 description "two words"#).unwrap_err();
        assert!(open.contains("unclosed \" quote"), "{open}");
        // Unquoted, a space still splits the value from the next group.
        assert!(parse("writem 100 av:1 description two words").is_err());
    }

    #[test]
    fn watch_lines_name_a_device_an_object_and_maybe_a_property() {
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        assert_eq!(
            parse("watch 100 ai:1"),
            Ok(Line::Watch(Point {
                device: 100,
                network: None,
                object: ai1,
                property: PropertyIdentifier::PRESENT_VALUE
            }))
        );
        assert_eq!(
            parse("w 2:100 analog-input:1 sf"),
            Ok(Line::Watch(Point {
                device: 100,
                network: Some(2),
                object: ai1,
                property: PropertyIdentifier::STATUS_FLAGS
            }))
        );
        assert_eq!(parse("csv on"), Ok(Line::Csv(true)));
        assert_eq!(
            parse("bbmd 10.0.0.1"),
            Ok(Line::Bbmd(Some("10.0.0.1:47808".parse().unwrap())))
        );
        assert!(parse("bbmd 10.0.0").is_err());
        assert_eq!(parse("bbmd"), Ok(Line::Bbmd(None)));
        assert!(
            parse("watch 10.0.0.5 ai:1")
                .unwrap_err()
                .contains("a device from the device table")
        );
        assert!(parse("watch 100 ai:1 object-list[2]").is_err());
        assert!(parse("watch 100 ai:x").is_err());
        assert_eq!(parse("find"), Ok(Line::Find(String::new())));
        let ms = Duration::from_millis;
        assert_eq!(
            parse("mstp rotation 1500"),
            Ok(Line::Mstp(Setting::Rotation(ms(1500))))
        );
        assert_eq!(
            parse("mstp hold 250"),
            Ok(Line::Mstp(Setting::Hold(ms(250))))
        );
        assert_eq!(
            parse("mstp listen 30"),
            Ok(Line::Mstp(Setting::Listen(Duration::from_secs(30))))
        );
        assert!(parse("mstp hold 0").is_err());
        assert!(parse("mstp listen 4000").is_err());
        assert!(parse("mstp poll 5").is_err());
        assert!(parse("finder x").unwrap_err().contains("unknown command"));
        assert!(
            parse("read 100 ai:1")
                .unwrap_err()
                .contains("unknown command")
        );
    }

    #[test]
    fn mstp_scan_commands_require_an_explicit_bounded_window() {
        for text in ["mstp scan 1", "mstp scan 3600", "mstp scan cancel"] {
            assert!(parse(text).is_ok(), "{text}: {:?}", parse(text));
        }
        for text in [
            "mstp scan",
            "mstp scan 0",
            "mstp scan 3601",
            "mstp scan -1",
            "mstp scan 1.5",
            "mstp scan cancel now",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn writem_lines_take_object_property_value_groups() {
        let Ok(Line::Writem(specs)) = parse("writem 100 ao:1 pv 42@8 av:1 description \"x\"")
        else {
            panic!("expected writem");
        };
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].value.as_deref(), Some("42"));
        assert_eq!(specs[0].priority, Some(8));
        assert_eq!(specs[1].property, PropertyIdentifier::DESCRIPTION);
        assert_eq!(specs[1].priority, None);
        assert!(parse("writem 100 ao:1 pv").is_err());
        assert!(parse("writem 100 ao:1 pv 42@17").is_err());
        assert!(parse("writem 100").is_err());
    }

    /// The table row a parsed line came from. A new `Line` fails to
    /// compile here until it names its row.
    fn row_of(line: &Line) -> &'static str {
        match line {
            Line::Watch(_) => "watch",
            Line::Csv(_) => "csv",
            Line::Writem(_) => "writem",
            Line::Bbmd(_) => "bbmd",
            Line::Find(_) => "find",
            Line::Mstp(_) | Line::Scan(_) => "mstp",
            Line::Manage(Action::Dcc { .. }) => "dcc",
            Line::Manage(Action::Reinit { .. }) => "reinit",
        }
    }

    /// Every parsed command has a row, so a hint and a help line; every
    /// row parses; and the hint for a line is its own row's.
    #[test]
    fn every_command_has_its_hint_from_the_table_the_parser_reads() {
        let samples = [
            "watch 100 ai:1",
            "w 100 ai:1 pv",
            "csv on",
            "writem 100 ao:1 pv 42@8",
            "bbmd",
            "bbmd 10.0.0.1",
            "find Supply Temp",
            "mstp hold 250",
            "mstp listen 30",
            "dcc disable-initiation 30",
            "control enable",
            "reinit warmstart",
        ];
        let mut rows = std::collections::BTreeSet::new();
        for sample in samples {
            let line = parse(sample).unwrap_or_else(|e| panic!("{sample}: {e}"));
            let row = row_of(&line);
            assert!(command(row).is_some(), "{row} has a row");
            let hint = matching(sample);
            assert_eq!(hint.len(), 1, "{sample}");
            assert_eq!(hint[0].name, row, "{sample}");
            rows.insert(row);
        }
        let names: std::collections::BTreeSet<&str> = COMMANDS.iter().map(|s| s.name).collect();
        assert_eq!(rows, names, "every row is parsed by a sample");
        for syntax in COMMANDS {
            for form in syntax.usage {
                assert_eq!(form.split_whitespace().next(), Some(syntax.name), "{form}");
            }
            assert!(usage().contains(syntax.usage[0]));
        }
        // A wrong form names its command's usage.
        assert_eq!(parse("csv maybe"), Err("usage: csv on|off".into()));
    }

    #[test]
    fn hints_follow_what_is_typed() {
        let names = |text: &str| -> Vec<&str> { matching(text).iter().map(|s| s.name).collect() };
        assert_eq!(names("").len(), COMMANDS.len());
        assert_eq!(names("w"), ["watch", "writem"]);
        assert_eq!(names("w "), ["watch"]);
        assert_eq!(names("wr"), ["writem"]);
        assert_eq!(names("mstp hold"), ["mstp"]);
        assert!(names("zz").is_empty());
        assert!(names("zz 1").is_empty());
    }
}
