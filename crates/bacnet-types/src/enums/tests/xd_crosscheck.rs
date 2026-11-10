//! Cross-check of our enumeration and bit-string tables against ASHRAE's
//! machine-readable BACnet datatypes (#2435).
//!
//! `testdata/ashrae-xd/xd-data-types.txt` is a stripped copy of ASHRAE's XD
//! data-types file (names and values only, Apache-2.0), regenerated with
//! `scripts/xd_strip.py`. It is a cross-check, not a source: the tables stay
//! hand-written. Every table we define is compared with its counterpart in the
//! file in both directions, names compared the way `FromStr` compares them
//! (`names_eq`). Each accepted difference is an entry in `xd_allowlist.rs`; the
//! test fails on a difference without an entry and on an entry that no longer
//! applies, so the list only shrinks as tables catch up.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::super::*;
use super::xd_allowlist::{Allowed, Category, allowed};
use crate::bitstring::{DaysOfWeek, EventTransitionBits, LimitEnable, LogStatus, StatusFlags};

/// Name and value pairs of one table; a bit string's values are bit positions.
type Members = Vec<(String, u32)>;

/// The file's tables by path (`0-BACnetAbortReason`, `0-Error/error-code`).
type XdTables = BTreeMap<String, Members>;

/// The file's path for our tables that don't follow `0-BACnet<OurName>`. A
/// type listed twice is compared with both tables.
const XD_PATHS: &[(&str, &str)] = &[
    (
        "AccessRuleLocationSpecifier",
        "0-BACnetAccessRule/location-specifier",
    ),
    (
        "AccessRuleTimeRangeSpecifier",
        "0-BACnetAccessRule/time-range-specifier",
    ),
    (
        "AcknowledgmentFilter",
        "0-GetEnrollmentSummary-Request/acknowledgment-filter",
    ),
    ("BackupAndRestoreState", "0-BACnetBackupState"),
    (
        "EnableDisable",
        "0-DeviceCommunicationControl-Request/enable-disable",
    ),
    (
        "EnrollmentSummaryEventStateFilter",
        "0-GetEnrollmentSummary-Request/event-state-filter",
    ),
    ("ErrorClass", "0-Error/error-class"),
    ("ErrorCode", "0-Error/error-code"),
    (
        "MessagePriority",
        "0-ConfirmedTextMessage-Request/message-priority",
    ),
    (
        "MessagePriority",
        "0-UnconfirmedTextMessage-Request/message-priority",
    ),
    ("ProgramChange", "0-BACnetProgramRequest"),
    (
        "ReinitializedState",
        "0-ReinitializeDevice-Request/reinitialized-state-of-device",
    ),
    // The enum names the bits of the ServicesSupported bit string.
    ("ServiceSupported", "0-BACnetServicesSupported"),
];

/// Every `bacnet_enum!` table, by type name.
macro_rules! enum_tables {
    ($($ty:ident),* $(,)?) => {
        vec![$((
            stringify!($ty),
            $ty::ALL_NAMED
                .iter()
                .map(|&(name, value)| (name.to_owned(), u32::from(value.to_raw())))
                .collect::<Members>(),
        )),*]
    };
}

/// Bit names of a `bitflags` type whose bit 0 is the string's first bit, or,
/// with `width`, whose top bit of `width` is the string's first bit.
fn flag_bits<T: bitflags::Flags>(width: Option<u32>) -> Members
where
    T::Bits: Into<u32>,
{
    T::FLAGS
        .iter()
        .map(|flag| {
            let low = flag.value().bits().into().trailing_zeros();
            let bit = width.map_or(low, |w| w - 1 - low);
            (flag.name().to_owned(), bit)
        })
        .collect()
}

/// All of our tables. `bitflags_types_are_all_registered` and
/// `enum_types_are_all_registered` keep these lists complete.
fn our_tables() -> Vec<(&'static str, Members)> {
    let mut tables = enum_tables![
        AbortReason,
        AccessAuthenticationFactorDisable,
        AccessCredentialDisable,
        AccessCredentialDisableReason,
        AccessEvent,
        AccessPassbackMode,
        AccessRuleLocationSpecifier,
        AccessRuleTimeRangeSpecifier,
        AccessUserType,
        AccessZoneOccupancyState,
        AcknowledgmentFilter,
        Action,
        AuditLevel,
        AuditOperation,
        AuthenticationFactorType,
        AuthenticationStatus,
        AuthorizationExemption,
        AuthorizationMode,
        BackupAndRestoreState,
        BACnetSuccessFilter,
        BinaryLightingPV,
        BinaryPV,
        ColorOperation,
        ColorOperationInProgress,
        ColorTransition,
        ConfirmedServiceChoice,
        DeviceStatus,
        DoorAlarmState,
        DoorSecuredStatus,
        DoorStatus,
        DoorValue,
        EnableDisable,
        EngineeringUnits,
        EnrollmentSummaryEventStateFilter,
        ErrorClass,
        ErrorCode,
        EscalatorFault,
        EscalatorMode,
        EscalatorOperationDirection,
        EventState,
        EventType,
        FaultType,
        FileAccessMethod,
        IPMode,
        LifeSafetyMode,
        LifeSafetyOperation,
        LifeSafetyState,
        LiftCarDirection,
        LiftCarDoorCommand,
        LiftCarDriveStatus,
        LiftCarMode,
        LiftFault,
        LiftGroupMode,
        LightingInProgress,
        LightingOperation,
        LightingTransition,
        LockStatus,
        LoggingType,
        Maintenance,
        MessagePriority,
        NetworkNumberQuality,
        NetworkPortCommand,
        NetworkReachability,
        NetworkType,
        NodeType,
        NotifyType,
        ObjectType,
        PduType,
        Polarity,
        ProgramChange,
        ProgramError,
        ProgramState,
        PropertyIdentifier,
        ProtocolLevel,
        ReinitializedState,
        RejectReason,
        Relationship,
        Reliability,
        RestartReason,
        Segmentation,
        ServiceSupported,
        ShedState,
        SilencedState,
        TimerState,
        TimerTransition,
        UnconfirmedServiceChoice,
        VTClass,
        WriteStatus,
    ];
    tables.extend([
        ("DaysOfWeek", flag_bits::<DaysOfWeek>(None)),
        (
            "EventTransitionBits",
            flag_bits::<EventTransitionBits>(None),
        ),
        ("LimitEnable", flag_bits::<LimitEnable>(None)),
        ("LogStatus", flag_bits::<LogStatus>(None)),
        // StatusFlags keeps the string's first bit at the top of four.
        ("StatusFlags", flag_bits::<StatusFlags>(Some(4))),
    ]);
    // These bit strings name their bits after an enumeration.
    for (bits, names) in [
        ("AuditOperationFlags", "AuditOperation"),
        ("ObjectTypesSupported", "ObjectType"),
    ] {
        let members = tables.iter().find(|(n, _)| *n == names).unwrap().1.clone();
        tables.push((bits, members));
    }
    tables
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Parse the stripped file: `enum|bits <path> [attr=N ...]` starts a table,
/// `<value> <name>` adds a member, `#` lines and blank lines are skipped.
fn parse_xd(text: &str) -> XdTables {
    let mut tables = XdTables::new();
    let mut current: Option<String> = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split(' ');
        let first = words.next().unwrap();
        if first == "enum" || first == "bits" {
            let path = words
                .next()
                .unwrap_or_else(|| panic!("line {}: no path", n + 1));
            assert!(
                tables.insert(path.to_owned(), Vec::new()).is_none(),
                "line {}: {path} repeats",
                n + 1
            );
            current = Some(path.to_owned());
            continue;
        }
        let value: u32 = first
            .parse()
            .unwrap_or_else(|_| panic!("line {}: not a member: {line}", n + 1));
        let name = words
            .next()
            .unwrap_or_else(|| panic!("line {}: no name", n + 1));
        assert!(words.next().is_none(), "line {}: extra words", n + 1);
        let table = current
            .as_ref()
            .unwrap_or_else(|| panic!("line {}: member before a table", n + 1));
        tables
            .get_mut(table)
            .unwrap()
            .push((name.to_owned(), value));
    }
    tables
}

/// One difference between a table of ours and the file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Diff {
    /// The file names `value`; we don't.
    Missing {
        table: String,
        value: u32,
        xd: String,
    },
    /// Both name `value`, differently.
    Renamed {
        table: String,
        value: u32,
        ours: String,
        xd: String,
    },
    /// We name `value`; the file doesn't.
    Extra {
        table: String,
        value: u32,
        ours: String,
    },
    /// A table (ours, or the file's path) with no counterpart.
    Unmatched { table: String },
}

fn xd_paths_for(ours: &str, xd: &XdTables) -> Vec<String> {
    let listed: Vec<String> = XD_PATHS
        .iter()
        .filter(|(o, _)| *o == ours)
        .map(|(_, p)| (*p).to_owned())
        .collect();
    if !listed.is_empty() {
        for p in &listed {
            assert!(
                xd.contains_key(p),
                "XD_PATHS names {p}, which the file lacks"
            );
        }
        return listed;
    }
    let auto = format!("0-BACnet{}", ours.strip_prefix("BACnet").unwrap_or(ours));
    if xd.contains_key(&auto) {
        vec![auto]
    } else {
        Vec::new()
    }
}

fn compare(table: &str, ours: &Members, theirs: &Members, out: &mut BTreeSet<Diff>) {
    for (xd_name, value) in theirs {
        let at_value: Vec<&String> = ours
            .iter()
            .filter(|(_, v)| v == value)
            .map(|(n, _)| n)
            .collect();
        if at_value.is_empty() {
            out.insert(Diff::Missing {
                table: table.into(),
                value: *value,
                xd: xd_name.clone(),
            });
        } else if !at_value.iter().any(|n| names_eq(n, xd_name)) {
            for n in at_value {
                out.insert(Diff::Renamed {
                    table: table.into(),
                    value: *value,
                    ours: n.clone(),
                    xd: xd_name.clone(),
                });
            }
        }
    }
    for (name, value) in ours {
        if !theirs.iter().any(|(_, v)| v == value) {
            out.insert(Diff::Extra {
                table: table.into(),
                value: *value,
                ours: name.clone(),
            });
        }
    }
}

/// Our member names that the file uses for a different value: a meaning
/// difference, never something to allow-list.
fn moved_names(table: &str, ours: &Members, theirs: &Members) -> Vec<String> {
    let mut moved = Vec::new();
    for (name, value) in ours {
        for (xd_name, xd_value) in theirs {
            if names_eq(name, xd_name) && value != xd_value {
                moved.push(format!(
                    "{table}: {name} is {value} here, {xd_value} in the file"
                ));
            }
        }
    }
    moved
}

fn allows(entry: &Allowed, diff: &Diff) -> bool {
    match (entry.category, diff) {
        (Category::LaterAddition | Category::FileDuplicate, Diff::Missing { table, value, .. })
        | (Category::OursOnly, Diff::Extra { table, value, .. }) => {
            entry.table == table && entry.values.contains(value)
        }
        (
            Category::OlderName | Category::LaterRename,
            Diff::Renamed {
                table,
                value,
                ours,
                xd,
            },
        ) => {
            entry.table == table
                && entry.values.contains(value)
                && entry.ours == ours
                && entry.xd == xd
        }
        (Category::UnmatchedType, Diff::Unmatched { table }) => entry.table == table,
        _ => false,
    }
}

/// The members a range entry speaks for: every one must be a difference.
fn covered_members<'a>(
    entry: &Allowed,
    ours: &'a BTreeMap<&str, Members>,
    xd: &'a XdTables,
) -> Vec<u32> {
    let pick = |m: &Members| {
        m.iter()
            .map(|(_, v)| *v)
            .filter(|v| entry.values.contains(v))
            .collect()
    };
    match entry.category {
        Category::LaterAddition | Category::FileDuplicate => {
            let paths = xd_paths_for(entry.table, xd);
            paths.first().map(|p| pick(&xd[p])).unwrap_or_default()
        }
        Category::OursOnly => ours.get(entry.table).map(pick).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[test]
fn tables_agree_with_ashrae_xd_except_where_allowed() {
    let root = workspace_root();
    // The data isn't in the published crate: built outside the repository,
    // there is nothing to compare against. Inside it, a missing file fails.
    if !root.join("crates/bacnet-types/Cargo.toml").is_file() {
        println!("XD cross-check skipped: not built inside the repository");
        return;
    }
    let path = root.join("testdata/ashrae-xd/xd-data-types.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let xd = parse_xd(&text);
    let ours: BTreeMap<&str, Members> = our_tables().into_iter().collect();

    let mut diffs = BTreeSet::new();
    let mut moved = Vec::new();
    let mut claimed = BTreeSet::new();
    for (name, members) in &ours {
        let paths = xd_paths_for(name, &xd);
        if paths.is_empty() {
            diffs.insert(Diff::Unmatched {
                table: (*name).to_owned(),
            });
        }
        for p in paths {
            compare(name, members, &xd[&p], &mut diffs);
            moved.extend(moved_names(name, members, &xd[&p]));
            claimed.insert(p);
        }
    }
    for p in xd.keys().filter(|p| !claimed.contains(*p)) {
        diffs.insert(Diff::Unmatched { table: p.clone() });
    }
    assert!(
        moved.is_empty(),
        "names whose value differs from the file:\n{}",
        moved.join("\n")
    );

    let allowed = allowed();
    let mut report = String::new();
    let unlisted: Vec<&Diff> = diffs
        .iter()
        .filter(|d| !allowed.iter().any(|e| allows(e, d)))
        .collect();
    for d in &unlisted {
        writeln!(report, "not on the allow-list: {d:?}").unwrap();
    }
    for entry in &allowed {
        if entry.why.trim().is_empty() {
            writeln!(report, "allow-list entry without a reason: {entry:?}").unwrap();
        }
        let used = diffs.iter().filter(|d| allows(entry, d)).count();
        let partial: Vec<u32> = covered_members(entry, &ours, &xd)
            .into_iter()
            .filter(|v| {
                !diffs
                    .iter()
                    .any(|d| allows(entry, d) && diff_value(d) == Some(*v))
            })
            .collect();
        if used == 0 {
            writeln!(report, "allow-list entry no longer applies: {entry:?}").unwrap();
        } else if !partial.is_empty() {
            writeln!(
                report,
                "allow-list entry covers values that now agree {partial:?}: {entry:?}"
            )
            .unwrap();
        }
    }

    let mut by_category: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for entry in &allowed {
        let slot = by_category
            .entry(format!("{:?}", entry.category))
            .or_default();
        slot.0 += 1;
        slot.1 += diffs.iter().filter(|d| allows(entry, d)).count();
    }
    println!(
        "XD cross-check: {} of our tables, {} in the file",
        ours.len(),
        xd.len()
    );
    for (category, (entries, values)) in &by_category {
        println!("  {category}: {entries} entries, {values} differences");
    }
    assert!(report.is_empty(), "{report}");
}

fn diff_value(d: &Diff) -> Option<u32> {
    match d {
        Diff::Missing { value, .. } | Diff::Renamed { value, .. } | Diff::Extra { value, .. } => {
            Some(*value)
        }
        Diff::Unmatched { .. } => None,
    }
}

/// The type names declared by `<macro>! { ... pub struct Name<end>` in `src/`.
fn declared(invocation: &str, end: char) -> BTreeSet<String> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut names = BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (at, _) in text.match_indices(invocation) {
            let rest = &text[at..];
            let start = rest
                .find("pub struct ")
                .expect("a struct in the invocation")
                + "pub struct ".len();
            let name: String = rest[start..].chars().take_while(|c| *c != end).collect();
            names.insert(name.trim().to_owned());
        }
    }
    names
}

#[test]
fn enum_types_are_all_registered() {
    // Spelled in two pieces so this file doesn't match itself.
    let declared = declared(concat!("bacnet_enum", "! {"), '(');
    let ours: BTreeSet<String> = our_tables()
        .into_iter()
        .map(|(n, _)| n.to_owned())
        .collect();
    let missing: Vec<&String> = declared.iter().filter(|n| !ours.contains(*n)).collect();
    assert!(missing.is_empty(), "add to our_tables(): {missing:?}");
    assert!(
        declared.len() > 80,
        "the source scan found only {declared:?}"
    );
}

#[test]
fn bitflags_types_are_all_registered() {
    let declared = declared(concat!("bitflags::bitflags", "! {"), ':');
    let ours: BTreeSet<String> = our_tables()
        .into_iter()
        .map(|(n, _)| n.to_owned())
        .collect();
    let missing: Vec<&String> = declared.iter().filter(|n| !ours.contains(*n)).collect();
    assert!(missing.is_empty(), "add to our_tables(): {missing:?}");
    assert!(
        declared.len() >= 5,
        "the source scan found only {declared:?}"
    );
}
