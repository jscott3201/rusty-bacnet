//! Cross-check of the standard value types against ASHRAE's machine-readable
//! BACnet object types (#2452).
//!
//! `testdata/ashrae-xd/xd-object-types.txt` is a stripped copy of ASHRAE's XD
//! object-types file (each object type's properties with their datatype,
//! array size and flags; Apache-2.0), regenerated with `scripts/xd_strip.py`.
//! It is a cross-check, not a source: the tables in `standard.rs` stay
//! hand-written. Every property row of every object type in the file is
//! compared with [`standard_value_type`] for that object type, and every
//! entry of [`by_type`] and [`by_identifier`] with the rows of the file, so a
//! difference shows from either side. The file's datatype names are mapped
//! onto [`ValueType`] by [`xd_value_type`], explicitly, name by name.
//!
//! Each accepted difference is an entry in `xd_allowlist.rs`. The test fails
//! on a difference without an entry, on an entry that no longer applies or
//! names a pair that now agrees, and on a datatype name the mapping doesn't
//! know.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::super::{ArraySize, Datatype as D, ValueShape, ValueType};
use super::xd_allowlist::{Allowed, Category, allowed};
use super::{by_identifier, by_type, standard_value_type};
use crate::enums::{ObjectType, PropertyIdentifier};

/// One property row of the file.
#[derive(Debug)]
struct XdRow {
    /// The datatype as the file writes it: `Real`, `Enumerated:0-BACnetX`,
    /// `Array<0-BACnetTimeStamp>`.
    datatype: String,
    /// A fixed-size array's size.
    size: Option<u16>,
}

/// The file's object types, by our object type, each with its rows by
/// property identifier.
type XdObjects = BTreeMap<ObjectType, BTreeMap<u32, XdRow>>;

/// Our name, in the kebab form the file and the allow-list use.
fn kebab(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

/// An object type's name in the allow-list: our name in kebab form.
fn object_name(object_type: ObjectType) -> String {
    object_type
        .name()
        .map_or_else(|| object_type.to_raw().to_string(), kebab)
}

/// A property's name in the allow-list: our name in kebab form, or its
/// number when we don't name it.
fn property_name(property: PropertyIdentifier) -> String {
    property
        .name()
        .map_or_else(|| property.to_raw().to_string(), kebab)
}

/// Letters and digits only, lowercased: `0-MultistateInputObject` and
/// `MULTI_STATE_INPUT` meet as `multistateinput`.
fn folded(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Our object type for the file's object name (`0-AnalogInputObject`).
fn object_type_for(xd_name: &str) -> Option<ObjectType> {
    let bare = xd_name.strip_prefix("0-")?.strip_suffix("Object")?;
    ObjectType::ALL_NAMED
        .iter()
        .find(|(name, _)| folded(name) == folded(bare))
        .map(|&(_, object_type)| object_type)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Parse the stripped object-types file: `object <name>` starts an object,
/// `<identifier> <name> <datatype> [key=value ...]` adds a row, `#` lines
/// and blank lines are skipped.
fn parse_objects(text: &str) -> XdObjects {
    let mut objects = XdObjects::new();
    let mut current = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split(' ');
        let first = words.next().unwrap();
        if first == "object" {
            let name = words
                .next()
                .unwrap_or_else(|| panic!("line {}: no name", n + 1));
            let object_type = object_type_for(name)
                .unwrap_or_else(|| panic!("line {}: no object type of ours for {name}", n + 1));
            assert!(
                objects.insert(object_type, BTreeMap::new()).is_none(),
                "line {}: {name} repeats",
                n + 1
            );
            current = Some(object_type);
            continue;
        }
        let id: u32 = first
            .parse()
            .unwrap_or_else(|_| panic!("line {}: not a property row: {line}", n + 1));
        let _name = words
            .next()
            .unwrap_or_else(|| panic!("line {}: no name", n + 1));
        let datatype = words
            .next()
            .unwrap_or_else(|| panic!("line {}: no datatype", n + 1))
            .to_owned();
        let mut size = None;
        for flag in words {
            let (key, value) = flag
                .split_once('=')
                .unwrap_or_else(|| panic!("line {}: not a flag: {flag}", n + 1));
            if key == "size" {
                size = Some(value.parse().unwrap());
            }
        }
        let object = current.unwrap_or_else(|| panic!("line {}: row before an object", n + 1));
        let rows = objects.get_mut(&object).unwrap();
        assert!(
            rows.insert(id, XdRow { datatype, size }).is_none(),
            "line {}: property {id} repeats",
            n + 1
        );
    }
    objects
}

/// The enumeration and bit-string paths of the stripped data-types file.
fn named_tables(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut enums, mut bits) = (BTreeSet::new(), BTreeSet::new());
    for line in text.lines() {
        let mut words = line.split(' ');
        match (words.next(), words.next()) {
            (Some("enum"), Some(path)) => enums.insert(path.to_owned()),
            (Some("bits"), Some(path)) => bits.insert(path.to_owned()),
            _ => false,
        };
    }
    (enums, bits)
}

/// The file's application primitives, and the datatype each is here. The
/// date and time patterns are the same encodings as Date, Time and
/// BACnetDateTime, which allow wildcard fields anyway; CharacterString is
/// `String` in the file.
const PRIMITIVES: &[(&str, D)] = &[
    ("Any", D::Any),
    ("BitString", D::BitString),
    ("Boolean", D::Boolean),
    ("Date", D::Date),
    ("DatePattern", D::Date),
    ("DateTime", D::DateTime),
    ("DateTimePattern", D::DateTime),
    ("Double", D::Double),
    ("Integer", D::Integer),
    ("ObjectIdentifier", D::ObjectIdentifier),
    ("OctetString", D::OctetString),
    ("Real", D::Real),
    ("String", D::CharacterString),
    ("Time", D::Time),
    ("TimePattern", D::Time),
    ("Unsigned", D::Unsigned),
];

/// The file's Unsigned subranges: an Unsigned here.
const UNSIGNED_RANGES: &[&str] = &[
    "0-BACnetAccessThreatLevel",
    "0-Unsigned8",
    "0-Unsigned16",
    "0-Unsigned32",
    "0-Unsigned64",
];

/// The file's constructed productions (SEQUENCE and CHOICE) and the datatype
/// each is here, one to one. An enumeration or a bit string needs no entry:
/// the data-types file says which names are those.
const CONSTRUCTED: &[(&str, D)] = &[
    ("0-BACnetAccessRule", D::AccessRule),
    ("0-BACnetActionList", D::ActionList),
    ("0-BACnetAddressBinding", D::AddressBinding),
    ("0-BACnetAssignedAccessRights", D::AssignedAccessRights),
    ("0-BACnetAssignedLandingCalls", D::AssignedLandingCalls),
    ("0-BACnetAuditLogRecord", D::AuditLogRecord),
    ("0-BACnetAuthenticationFactor", D::AuthenticationFactor),
    (
        "0-BACnetAuthenticationFactorFormat",
        D::AuthenticationFactorFormat,
    ),
    ("0-BACnetAuthenticationPolicy", D::AuthenticationPolicy),
    ("0-BACnetBDTEntry", D::BdtEntry),
    ("0-BACnetCalendarEntry", D::CalendarEntry),
    ("0-BACnetChannelValue", D::ChannelValue),
    ("0-BACnetClientCOV", D::ClientCov),
    ("0-BACnetColorCommand", D::ColorCommand),
    (
        "0-BACnetCOVMultipleSubscription",
        D::CovMultipleSubscription,
    ),
    ("0-BACnetCOVSubscription", D::CovSubscription),
    (
        "0-BACnetCredentialAuthenticationFactor",
        D::CredentialAuthenticationFactor,
    ),
    ("0-BACnetDailySchedule", D::DailySchedule),
    ("0-BACnetDateRange", D::DateRange),
    ("0-BACnetDestination", D::Destination),
    (
        "0-BACnetDeviceObjectPropertyReference",
        D::DeviceObjectPropertyReference,
    ),
    ("0-BACnetDeviceObjectReference", D::DeviceObjectReference),
    ("0-BACnetEventLogRecord", D::EventLogRecord),
    (
        "0-BACnetEventNotificationSubscription",
        D::EventNotificationSubscription,
    ),
    ("0-BACnetEventParameter", D::EventParameter),
    ("0-BACnetFaultParameter", D::FaultParameter),
    ("0-BACnetFDTEntry", D::FdtEntry),
    ("0-BACnetHostNPort", D::HostNPort),
    ("0-BACnetLandingCallStatus", D::LandingCallStatus),
    ("0-BACnetLandingDoorStatus", D::LandingDoorStatus),
    ("0-BACnetLiftCarCallList", D::LiftCarCallList),
    ("0-BACnetLightingCommand", D::LightingCommand),
    ("0-BACnetLogMultipleRecord", D::LogMultipleRecord),
    ("0-BACnetLogRecord", D::LogRecord),
    ("0-BACnetNameValue", D::NameValue),
    ("0-BACnetNameValueCollection", D::NameValueCollection),
    (
        "0-BACnetObjectPropertyReference",
        D::ObjectPropertyReference,
    ),
    ("0-BACnetObjectSelector", D::ObjectSelector),
    ("0-BACnetOptionalBinaryPV", D::OptionalBinaryPv),
    (
        "0-BACnetOptionalCharacterString",
        D::OptionalCharacterString,
    ),
    ("0-BACnetOptionalPriorityFilter", D::OptionalPriorityFilter),
    ("0-BACnetOptionalReal", D::OptionalReal),
    ("0-BACnetOptionalUnsigned", D::OptionalUnsigned),
    ("0-BACnetPortPermission", D::PortPermission),
    ("0-BACnetPrescale", D::Prescale),
    ("0-BACnetProcessIdSelection", D::ProcessIdSelection),
    ("0-BACnetPropertyAccessResult", D::PropertyAccessResult),
    ("0-BACnetRecipient", D::Recipient),
    ("0-BACnetRouterEntry", D::RouterEntry),
    ("0-BACnetScale", D::Scale),
    ("0-BACnetSetpointReference", D::SetpointReference),
    ("0-BACnetShedLevel", D::ShedLevel),
    ("0-BACnetSpecialEvent", D::SpecialEvent),
    ("0-BACnetStageLimitValue", D::StageLimitValue),
    ("0-BACnetTimerStateChangeValue", D::TimerStateChangeValue),
    ("0-BACnetTimeStamp", D::TimeStamp),
    ("0-BACnetValueSource", D::ValueSource),
    ("0-BACnetVMACEntry", D::VmacEntry),
    ("0-BACnetVTSession", D::VtSession),
    ("0-BACnetxyColor", D::XyColor),
    ("0-ReadAccessResult", D::ReadAccessResult),
    ("0-ReadAccessSpecification", D::ReadAccessSpecification),
];

/// The file's datatype names with no datatype here, and why. A row of one
/// can't be compared: its difference is on the allow-list.
const UNMAPPED: &[(&str, &str)] = &[
    (
        "0-BACnetAccumulatorRecord",
        "Logging_Record isn't modelled yet",
    ),
    ("0-BACnetHealth", "a port health row added after 2020"),
    ("0-BACnetOptionalBinaryLightingPV", PRIORITY_ELEMENT),
    ("0-BACnetOptionalBitString", PRIORITY_ELEMENT),
    ("0-BACnetOptionalDate", PRIORITY_ELEMENT),
    ("0-BACnetOptionalDatePattern", PRIORITY_ELEMENT),
    ("0-BACnetOptionalDateTime", PRIORITY_ELEMENT),
    ("0-BACnetOptionalDateTimePattern", PRIORITY_ELEMENT),
    ("0-BACnetOptionalDoorValue", PRIORITY_ELEMENT),
    ("0-BACnetOptionalDouble", PRIORITY_ELEMENT),
    ("0-BACnetOptionalInteger", PRIORITY_ELEMENT),
    ("0-BACnetOptionalOctetString", PRIORITY_ELEMENT),
    ("0-BACnetOptionalTime", PRIORITY_ELEMENT),
    ("0-BACnetOptionalTimePattern", PRIORITY_ELEMENT),
    ("0-BACnetSCDirectConnection", SC_ROW),
    ("0-BACnetSCFailedConnectionRequest", SC_ROW),
    ("0-BACnetSCHubConnection", SC_ROW),
    ("0-BACnetSCHubFunctionConnection", SC_ROW),
];

/// Used only as a Priority_Array element, where 2020 has BACnetPriorityValue.
const PRIORITY_ELEMENT: &str = "a Priority_Array element type; ours is PriorityValue";
/// Used only by the BACnet/SC port rows added after 2020.
const SC_ROW: &str = "a BACnet/SC port row added after 2020";

/// What the mapping makes of the file's names.
struct Mapping {
    enums: BTreeSet<String>,
    bits: BTreeSet<String>,
    /// Every name the mapping resolved, to find stale mapping entries.
    used: BTreeSet<String>,
    /// Every name it couldn't, to hold against [`UNMAPPED`].
    unmapped: BTreeSet<String>,
}

impl Mapping {
    /// The datatype of one element or value named `name`, or `None` for a
    /// name with no datatype here.
    fn datatype(&mut self, name: &str) -> Option<D> {
        let found = PRIMITIVES
            .iter()
            .chain(CONSTRUCTED)
            .find(|(n, _)| *n == name)
            .map(|&(_, d)| d)
            .or_else(|| UNSIGNED_RANGES.contains(&name).then_some(D::Unsigned))
            .or_else(|| self.enums.contains(name).then_some(D::Enumerated))
            .or_else(|| self.bits.contains(name).then_some(D::BitString));
        match found {
            Some(_) => self.used.insert(name.to_owned()),
            None => self.unmapped.insert(name.to_owned()),
        };
        found
    }
}

/// The value type a row of the file gives, or `None` when its datatype name
/// has no datatype here.
///
/// `Tag` is a primitive; `Tag:Name` a primitive of a named enumeration, bit
/// string or subrange, or the named SEQUENCE or CHOICE; `Array<Element>` and
/// `List<Element>` a BACnetARRAY or BACnetLIST of one, fixed-size when the
/// row gives a size.
fn xd_value_type(row: &XdRow, mapping: &mut Mapping) -> Option<ValueType> {
    let dt = row.datatype.as_str();
    fn element(inner: &str) -> &str {
        inner.strip_suffix('>').expect("a closing '>'")
    }
    if let Some(inner) = dt.strip_prefix("Array<") {
        let datatype = mapping.datatype(element(inner))?;
        return Some(match row.size {
            Some(size) => ValueType::fixed_array(datatype, size),
            None => ValueType::array(datatype),
        });
    }
    assert!(row.size.is_none(), "a size on {dt}, which isn't an array");
    if let Some(inner) = dt.strip_prefix("List<") {
        return mapping.datatype(element(inner)).map(ValueType::list);
    }
    let datatype = match dt.split_once(':') {
        Some(("Enumerated", name)) => {
            assert!(mapping.enums.contains(name), "{name} isn't an enumeration");
            mapping.used.insert(name.to_owned());
            D::Enumerated
        }
        Some(("BitString", name)) => {
            assert!(mapping.bits.contains(name), "{name} isn't a bit string");
            mapping.used.insert(name.to_owned());
            D::BitString
        }
        Some(("Unsigned", name)) => {
            assert_eq!(mapping.datatype(name), Some(D::Unsigned), "{dt}");
            D::Unsigned
        }
        Some(("Sequence" | "Choice", name)) => mapping.datatype(name)?,
        Some(_) => panic!("unknown datatype form {dt}"),
        None => mapping.datatype(dt)?,
    };
    Some(ValueType::single(datatype))
}

/// A value type as the allow-list writes it: `Real`, `Array<TimeStamp>[3]`,
/// `Array<CharacterString>`, `List<Recipient>`.
fn show(value_type: ValueType) -> String {
    let datatype = format!("{:?}", value_type.datatype);
    match value_type.shape {
        ValueShape::Single => datatype,
        ValueShape::Array(ArraySize::Fixed(size)) => format!("Array<{datatype}>[{size}]"),
        ValueShape::Array(ArraySize::Variable) => format!("Array<{datatype}>"),
        ValueShape::List => format!("List<{datatype}>"),
    }
}

/// A row's datatype as the allow-list writes it: the file's spelling, with
/// a fixed array's size after it.
fn show_xd(row: &XdRow) -> String {
    match row.size {
        Some(size) => format!("{}[{size}]", row.datatype),
        None => row.datatype.clone(),
    }
}

/// One difference between our tables and the file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Diff {
    /// The file lists the property on the object type; we have no value
    /// type for it there.
    NoType {
        object: String,
        property: String,
        xd: String,
    },
    /// Both give a value type and they differ, or the file's has no
    /// datatype here (`UNMAPPED`).
    Differs {
        object: String,
        property: String,
        ours: String,
        xd: String,
    },
    /// We give a value type for the property on the object type ([`by_type`]),
    /// or on every type ([`by_identifier`], `object` "*"), and the file lists
    /// the property on none of them.
    OursOnly { object: String, property: String },
}

impl Diff {
    fn key(&self) -> (&str, &str) {
        match self {
            Self::NoType {
                object, property, ..
            }
            | Self::Differs {
                object, property, ..
            }
            | Self::OursOnly { object, property } => (object, property),
        }
    }
}

fn allows(entry: &Allowed, diff: &Diff) -> bool {
    let (object, property) = diff.key();
    let kind_matches = match (entry.category, diff) {
        (Category::LaterAddition | Category::NotModelled, Diff::NoType { .. })
        | (Category::OursOnly, Diff::OursOnly { .. }) => true,
        (
            Category::NotOneToOne | Category::LaterChange | Category::ToFix,
            Diff::Differs { ours, xd, .. },
        ) => entry.ours == ours && entry.xd == xd,
        (Category::ToFix, Diff::NoType { .. } | Diff::OursOnly { .. }) => entry.ours.is_empty(),
        _ => false,
    };
    kind_matches && entry.objects.contains(&object) && entry.properties.contains(&property)
}

/// The differences, the mapping's record, and how many rows agree.
fn diffs() -> (BTreeSet<Diff>, Mapping, usize) {
    let root = workspace_root();
    let read = |name: &str| {
        let path = root.join("testdata/ashrae-xd").join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let xd = parse_objects(&read("xd-object-types.txt"));
    let (enums, bits) = named_tables(&read("xd-data-types.txt"));
    let mut mapping = Mapping {
        enums,
        bits,
        used: BTreeSet::new(),
        unmapped: BTreeSet::new(),
    };

    let mut diffs = BTreeSet::new();
    let mut agree = 0;
    for (&object_type, rows) in &xd {
        for (&id, row) in rows {
            let property = PropertyIdentifier::from_raw(id);
            let (object, property_shown) = (object_name(object_type), property_name(property));
            let theirs = xd_value_type(row, &mut mapping);
            match standard_value_type(object_type, property) {
                None => {
                    diffs.insert(Diff::NoType {
                        object,
                        property: property_shown,
                        xd: show_xd(row),
                    });
                }
                Some(ours) if theirs != Some(ours) => {
                    diffs.insert(Diff::Differs {
                        object,
                        property: property_shown,
                        ours: show(ours),
                        xd: show_xd(row),
                    });
                }
                Some(_) => agree += 1,
            }
        }
    }

    // The other direction: a type-dependent entry for a property the file's
    // object type lacks, or an identifier entry no object type in the file
    // has.
    for &(_, object_type) in ObjectType::ALL_NAMED {
        let rows = xd.get(&object_type);
        for &(_, property) in PropertyIdentifier::ALL_NAMED {
            let listed = rows.is_some_and(|r| r.contains_key(&property.to_raw()));
            if by_type(object_type, property).is_some() && !listed {
                diffs.insert(Diff::OursOnly {
                    object: object_name(object_type),
                    property: property_name(property),
                });
            }
        }
    }
    for &(_, property) in PropertyIdentifier::ALL_NAMED {
        let anywhere = xd.values().any(|r| r.contains_key(&property.to_raw()));
        if by_identifier(property).is_some() && !anywhere {
            diffs.insert(Diff::OursOnly {
                object: "*".to_owned(),
                property: property_name(property),
            });
        }
    }
    (diffs, mapping, agree)
}

#[test]
fn value_types_agree_with_ashrae_xd_except_where_allowed() {
    // The data isn't in the published crate: built outside the repository,
    // there is nothing to compare against. Inside it, a missing file fails.
    if !workspace_root()
        .join("crates/bacnet-types/Cargo.toml")
        .is_file()
    {
        println!("XD value-type cross-check skipped: not built inside the repository");
        return;
    }
    let (diffs, mapping, agree) = diffs();
    let allowed = allowed();
    let mut report = String::new();

    // The mapping: every name the file uses is mapped or listed as unmapped,
    // and every mapping entry is used.
    let listed: BTreeSet<&str> = UNMAPPED.iter().map(|(n, _)| *n).collect();
    for name in &mapping.unmapped {
        if !listed.contains(name.as_str()) {
            writeln!(report, "datatype name not mapped or listed: {name}").unwrap();
        }
    }
    for name in &listed {
        if !mapping.unmapped.contains(*name) {
            writeln!(report, "UNMAPPED entry no longer needed: {name}").unwrap();
        }
    }
    for (name, _) in PRIMITIVES.iter().chain(CONSTRUCTED) {
        if !mapping.used.contains(*name) {
            writeln!(report, "mapping entry the file never uses: {name}").unwrap();
        }
    }
    for name in UNSIGNED_RANGES {
        if !mapping.used.contains(*name) {
            writeln!(report, "mapping entry the file never uses: {name}").unwrap();
        }
    }

    for d in diffs
        .iter()
        .filter(|d| !allowed.iter().any(|e| allows(e, d)))
    {
        writeln!(report, "not on the allow-list: {d:?}").unwrap();
    }
    for entry in &allowed {
        if entry.why.trim().is_empty() {
            writeln!(report, "allow-list entry without a reason: {entry:?}").unwrap();
        }
        let agree: Vec<String> = entry
            .pairs()
            .filter(|(o, p)| {
                !diffs
                    .iter()
                    .any(|d| allows(entry, d) && d.key() == (o.as_str(), p.as_str()))
            })
            .map(|(o, p)| format!("{o} {p}"))
            .collect();
        if !agree.is_empty() {
            writeln!(
                report,
                "allow-list entry names pairs that no longer differ {agree:?}: {entry:?}"
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
        "XD value-type cross-check: {agree} rows agree, {} differences, {} datatype names mapped, {} unmapped",
        diffs.len(),
        mapping.used.len(),
        mapping.unmapped.len()
    );
    for (category, (entries, count)) in &by_category {
        println!("  {category}: {entries} entries, {count} differences");
    }
    assert!(report.is_empty(), "{report}");
}

#[test]
fn every_xd_object_type_is_ours() {
    if !workspace_root()
        .join("crates/bacnet-types/Cargo.toml")
        .is_file()
    {
        return;
    }
    let path = workspace_root().join("testdata/ashrae-xd/xd-object-types.txt");
    let xd = parse_objects(&std::fs::read_to_string(path).unwrap());
    // Network Security (38) is the one type of ours the file lacks; its
    // clause was removed before 2020.
    let missing: Vec<ObjectType> = ObjectType::ALL_NAMED
        .iter()
        .map(|&(_, t)| t)
        .filter(|t| !xd.contains_key(t))
        .collect();
    assert_eq!(missing, [ObjectType::NETWORK_SECURITY]);
    assert_eq!(xd.len(), 64);
}
