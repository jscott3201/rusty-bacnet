//! Accepted differences between our tables and ASHRAE's XD data-types file,
//! read by `xd_crosscheck.rs` (#2435).
//!
//! The file is protocol revision 24; this stack implements revision 22
//! (135-2020). Step 2 of #2435 named the later values and step 3 renamed our
//! identifiers to the 2020 names, so no `later-addition` or `older-name` entry
//! is left; the categories stay so that a new difference can be recorded.
//!
//! One entry per line: `<category> <table> <value>[..<last>] [<ours> <file's>] | <reason>`.
//! `<table>` is our type name, or the file's table path for a file table with
//! no counterpart. A range covers every value the relevant side names in it,
//! and each of those must still differ. The reason is in our own words.
//!
//! - `later-addition`: the file names values that 135-2020 doesn't, and we don't name them yet.
//! - `older-name`: our identifier differs from the 2020 name, which the file keeps.
//! - `later-rename`: our identifier is the 2020 name; the file carries a later rename (we keep ours).
//! - `ours-only`: we name values the file doesn't.
//! - `file-duplicate`: the file gives one value two names, so we leave it unnamed.
//! - `file-error`: the file also gives `<value>` a name that belongs to another value; we name
//!   both values as corrected, so the file's stray name is excused at its other value too.
//! - `unmatched-type`: a table with no counterpart on the other side (value 0, no names).

use core::ops::RangeInclusive;

pub(super) const ALLOWED: &str = "
later-rename EngineeringUnits 63 DEGREES_KELVIN kelvin | 2020 says degrees kelvin
later-rename EngineeringUnits 121 DELTA_DEGREES_KELVIN delta-kelvin | 2020 says degrees kelvin
later-rename EngineeringUnits 127 JOULES_PER_DEGREE_KELVIN joules-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 128 JOULES_PER_KILOGRAM_DEGREE_KELVIN joules-per-kilogram-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 141 WATTS_PER_SQUARE_METER_DEGREE_KELVIN watts-per-square-meter-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 151 KILOJOULES_PER_DEGREE_KELVIN kilojoules-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 152 MEGAJOULES_PER_DEGREE_KELVIN megajoules-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 176 VOLTS_PER_DEGREE_KELVIN volts-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 181 DEGREES_KELVIN_PER_HOUR kelvin-per-hour | 2020 says degrees kelvin
later-rename EngineeringUnits 182 DEGREES_KELVIN_PER_MINUTE kelvin-per-minute | 2020 says degrees kelvin
later-rename EngineeringUnits 189 WATTS_PER_METER_PER_DEGREE_KELVIN watts-per-meter-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 236 MINUTES_PER_DEGREE_KELVIN minutes-per-kelvin | 2020 says degree kelvin
later-rename EngineeringUnits 247 JOULE_PER_HOURS joules-per-hour | 2020 spells it this way
later-rename NetworkPortCommand 3 RESTART_SLAVE_DISCOVERY restart-subordinate-discovery | 2020's MS/TP role term
later-rename PropertyIdentifier 64 MAX_MASTER max-manager | 2020's MS/TP role term
later-rename PropertyIdentifier 169 AUTO_SLAVE_DISCOVERY auto-subordinate-discovery | 2020's MS/TP role term
later-rename PropertyIdentifier 170 MANUAL_SLAVE_ADDRESS_BINDING manual-subordinate-address-binding | 2020's MS/TP role term
later-rename PropertyIdentifier 171 SLAVE_ADDRESS_BINDING subordinate-address-binding | 2020's MS/TP role term
later-rename PropertyIdentifier 172 SLAVE_PROXY_ENABLE subordinate-proxy-enable | 2020's MS/TP role term

ours-only EnableDisable 1 | deprecated before 2020; kept to decode older requests
ours-only ErrorCode 1 | network security, which 2020 already removed
ours-only ErrorCode 6 | network security, which 2020 already removed
ours-only ErrorCode 12 | network security, which 2020 already removed
ours-only ErrorCode 15 | network security, which 2020 already removed
ours-only ErrorCode 28 | network security, which 2020 already removed
ours-only ErrorCode 90..94 | network security, which 2020 already removed
ours-only ErrorCode 98..100 | network security, which 2020 already removed
ours-only ErrorCode 102 | network security, which 2020 already removed
ours-only ErrorCode 105 | network security, which 2020 already removed
ours-only ErrorCode 107..109 | network security, which 2020 already removed
ours-only NetworkType 8 | withdrawn by revision 18, before 2020
ours-only ObjectType 38 | network security, which 2020 already removed
ours-only ObjectTypesSupported 38 | network security, which 2020 already removed
ours-only PropertyIdentifier 327..337 | network security, which 2020 already removed

file-error EngineeringUnits 47910 MILLISIEMENS_PER_CENTIMETER active-energy-pulse-value | the file lists the pulse-value name here as well; we keep that name at 47918
ours-only EngineeringUnits 47918 | the file lists this name at 47910 and no name at 47918

unmatched-type PduType 0 | the file describes PDUs as a choice of sequences, not an enumeration
unmatched-type 0-BACnetAccumulatorRecord/accumulator-status 0 | no table of ours yet
unmatched-type 0-BACnetPriorityFilter 0 | BACnetPriorityFilter has no per-bit names (bit N is priority N+1)
unmatched-type 0-BACnetResultFlags 0 | ReadRange result flags are three booleans in bacnet-services
unmatched-type 0-BACnetRouterEntry/status 0 | no table of ours; nothing reports a router entry status
";

/// Why a difference is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Category {
    LaterAddition,
    OlderName,
    LaterRename,
    OursOnly,
    FileDuplicate,
    FileError,
    UnmatchedType,
}

/// One accepted difference, or a run of them over `values`.
#[derive(Debug)]
pub(super) struct Allowed {
    pub category: Category,
    pub table: &'static str,
    pub values: RangeInclusive<u32>,
    /// For a name difference: our identifier and the file's name.
    pub ours: &'static str,
    pub xd: &'static str,
    pub why: &'static str,
}

/// Parse [`ALLOWED`], panicking on a malformed line.
pub(super) fn allowed() -> Vec<Allowed> {
    let mut entries = Vec::new();
    for line in ALLOWED.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (fields, why) = line
            .split_once(" | ")
            .unwrap_or_else(|| panic!("no reason: {line}"));
        let fields: Vec<&'static str> = fields.split(' ').collect();
        let category = match fields[0] {
            "later-addition" => Category::LaterAddition,
            "older-name" => Category::OlderName,
            "later-rename" => Category::LaterRename,
            "ours-only" => Category::OursOnly,
            "file-duplicate" => Category::FileDuplicate,
            "file-error" => Category::FileError,
            "unmatched-type" => Category::UnmatchedType,
            other => panic!("unknown category {other}: {line}"),
        };
        let names = matches!(
            category,
            Category::OlderName | Category::LaterRename | Category::FileError
        );
        assert_eq!(
            fields.len(),
            if names { 5 } else { 3 },
            "field count: {line}"
        );
        let number = |s: &str| {
            s.parse::<u32>()
                .unwrap_or_else(|_| panic!("number {s}: {line}"))
        };
        let values = match fields[2].split_once("..") {
            Some((lo, hi)) => number(lo)..=number(hi),
            None => number(fields[2])..=number(fields[2]),
        };
        assert!(!values.is_empty(), "empty range: {line}");
        entries.push(Allowed {
            category,
            table: fields[1],
            values,
            ours: if names { fields[3] } else { "" },
            xd: if names { fields[4] } else { "" },
            why: why.trim(),
        });
    }
    entries
}
