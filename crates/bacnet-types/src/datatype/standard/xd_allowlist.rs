//! Accepted differences between the standard value types and ASHRAE's XD
//! object-types file, read by `xd_crosscheck.rs` (#2452).
//!
//! The file is protocol revision 24; this stack implements revision 22
//! (135-2020).
//!
//! One entry per line: `<category> <objects> <properties> [<ours> <file's>] | <reason>`.
//! `<objects>` and `<properties>` are comma-separated lists in our names,
//! kebab-case (a property we don't name by its number; `*` as the object for
//! an entry of the by-identifier table). Every object and property pair an
//! entry names must still differ, in the way its category says. A datatype
//! difference also names both value types, as the test prints them. The
//! reason is in our own words.
//!
//! - `later-addition`: the file lists a property that 135-2020 doesn't define on that object type.
//! - `not-modelled`: 135-2020 defines the property there, and our tables have no value type for
//!   it. No built-in object serves one: bacnet-objects' `every_served_row_has_a_value_type`
//!   holds that, so these are gaps to fill as objects grow, not wrong answers.
//! - `later-change`: the file's datatype is a later revision's; ours is the 2020 one.
//! - `not-one-to-one`: both describe the same values, in different terms.
//! - `ours-only`: we give a value type for a property the file doesn't list there.
//! - `to-fix`: our table is wrong at revision 22; the reason names the follow-up.

pub(super) const ALLOWED: &str = "
later-addition network-port additional-reference-ports,certificate-signing-request-file,command-validation-result,issuer-certificate-files | Addendum 135-2020cc rows
later-addition network-port current-health,max-bvlc-length-accepted,max-npdu-length-accepted,operational-certificate-file,sc-connect-wait-timeout,sc-direct-connect-accept-enable,sc-direct-connect-accept-uris,sc-direct-connect-binding,sc-direct-connect-connection-status,sc-direct-connect-initiate-enable,sc-disconnect-wait-timeout,sc-failed-connection-requests,sc-failover-hub-connection-status,sc-failover-hub-uri,sc-heartbeat-timeout,sc-hub-connector-state,sc-hub-function-accept-uris,sc-hub-function-binding,sc-hub-function-connection-status,sc-hub-function-enable,sc-maximum-reconnect-time,sc-minimum-reconnect-time,sc-primary-hub-connection-status,sc-primary-hub-uri | BACnet/SC port rows added after 2020
later-addition schedule write-every-scheduled-action | added after 2020

not-modelled access-credential absentee-limit,belongs-to,days-remaining,extended-time-enable,last-access-event,last-access-point,last-use-time,threat-authority,trace-flag,uses-remaining | Table 12-40 rows
not-modelled access-door door-unlock-delay-time | Table 12-30 row
not-modelled access-point accompaniment-time,failed-attempts,failed-attempts-time,lockout,lockout-relinquish-time,max-failed-attempts,muster-point,occupancy-count-adjust,occupancy-lower-limit-enforced,occupancy-upper-limit-enforced,threat-level,transaction-notification-class,verification-time,zone-from,zone-to | Table 12-36 rows
not-modelled access-user user-external-identifier,user-information-reference,user-name | Table 12-38 rows
not-modelled access-zone last-credential-added,last-credential-added-time,last-credential-removed,last-credential-removed-time,passback-mode,passback-timeout | Table 12-37 rows
not-modelled accumulator logging-object,logging-record,value-change-time | Table 12-79 rows
not-modelled analog-input,analog-output,analog-value,integer-value,large-analog-value,positive-integer-value,timer resolution | Tables 12-2, 12-3, 12-4, 12-50, 12-46, 12-51 and 12-75 rows; the datatype follows Present_Value
not-modelled averaging maximum-value-timestamp,minimum-value-timestamp,variance-value | Table 12-5 rows
not-modelled binary-input change-of-state-count,change-of-state-time,elapsed-active-time,time-of-active-time-reset,time-of-state-count-reset | Table 12-6 rows
not-modelled binary-output,binary-value change-of-state-count,change-of-state-time,elapsed-active-time,minimum-off-time,minimum-on-time,time-of-active-time-reset,time-of-state-count-reset | Tables 12-8 and 12-10 rows
not-modelled binary-lighting-output elapsed-active-time,power,strike-count,time-of-active-time-reset,time-of-strike-count-reset | Table 12-69 rows
not-modelled bitstring-value bit-mask | Table 12-47 row
not-modelled datetime-value,datetimepattern-value is-utc | Tables 12-45 and 12-53 rows
not-modelled device backup-preparation-time,deployed-profile-location,max-info-frames,max-master,restore-completion-time,restore-preparation-time,serial-number,time-synchronization-interval | Table 12-13 rows
not-modelled global-group covu-period,requested-update-interval | Table 12-57 rows
not-modelled life-safety-point floor-number,setting | Table 12-18 rows
not-modelled life-safety-zone floor-number | Table 12-19 row
not-modelled lift higher-deck,lower-deck | Table 12-77 rows
not-modelled lighting-output instantaneous-power,max-actual-value,min-actual-value,power | Table 12-64 rows
not-modelled load-control full-duty-baseline,state-description | Table 12-32 rows
not-modelled loop bias,error-limit,maximum-output,minimum-output | Table 12-20 rows
not-modelled network-port auto-slave-discovery,command,ip-dhcp-enable,ip-dhcp-lease-time,ip-dhcp-lease-time-remaining,ip-dhcp-server,ipv6-auto-addressing-enable,ipv6-dhcp-lease-time,ipv6-dhcp-lease-time-remaining,ipv6-dhcp-server,ipv6-zone-index,link-speed-autonegotiate,max-info-frames,max-master,network-interface-name,reference-port,slave-proxy-enable | Table 12-71 rows
not-modelled program description-of-halt,instance-of,program-location | Table 12-26 rows
not-modelled staging default-present-value | Table 12-80 row
not-modelled structured-view default-subordinate-relationship,represents | Table 12-34 rows

not-one-to-one access-door priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalDoorValue>[16] | 2020's BACnetPriorityArray holds BACnetPriorityValue; the file names each type's own NULL-or-value form
not-one-to-one analog-output,analog-value,lighting-output priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalReal>[16] | as for access-door
not-one-to-one binary-lighting-output priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalBinaryLightingPV>[16] | as for access-door
not-one-to-one binary-output,binary-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalBinaryPV>[16] | as for access-door
not-one-to-one bitstring-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalBitString>[16] | as for access-door
not-one-to-one characterstring-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalCharacterString>[16] | as for access-door
not-one-to-one date-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalDate>[16] | as for access-door
not-one-to-one datepattern-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalDatePattern>[16] | as for access-door
not-one-to-one datetime-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalDateTime>[16] | as for access-door
not-one-to-one datetimepattern-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalDateTimePattern>[16] | as for access-door
not-one-to-one integer-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalInteger>[16] | as for access-door
not-one-to-one large-analog-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalDouble>[16] | as for access-door
not-one-to-one multi-state-output,multi-state-value,positive-integer-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalUnsigned>[16] | as for access-door
not-one-to-one octetstring-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalOctetString>[16] | as for access-door
not-one-to-one time-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalTime>[16] | as for access-door
not-one-to-one timepattern-value priority-array Array<PriorityValue>[16] Array<0-BACnetOptionalTimePattern>[16] | as for access-door

ours-only bitstring-value fault-values | Table 12-47 has no Fault_Values; the entry predates this check and shapes only a custom object's row

to-fix audit-reporter audit-priority-filter OptionalPriorityFilter BitString:0-BACnetPriorityFilter | Table 12-82 types it a plain BACnetPriorityFilter, but the by-identifier entry gives every type the optional form; follow-up to file
";

/// Why a difference is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Category {
    LaterAddition,
    NotModelled,
    LaterChange,
    NotOneToOne,
    OursOnly,
    ToFix,
}

/// One accepted difference, or the same one on several pairs.
#[derive(Debug)]
pub(super) struct Allowed {
    pub category: Category,
    pub objects: Vec<&'static str>,
    pub properties: Vec<&'static str>,
    /// For a datatype difference: our value type and the file's.
    pub ours: &'static str,
    pub xd: &'static str,
    pub why: &'static str,
}

impl Allowed {
    /// Every object and property pair the entry names.
    pub fn pairs(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.objects.iter().flat_map(|o| {
            self.properties
                .iter()
                .map(move |p| ((*o).to_owned(), (*p).to_owned()))
        })
    }
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
            "not-modelled" => Category::NotModelled,
            "later-change" => Category::LaterChange,
            "not-one-to-one" => Category::NotOneToOne,
            "ours-only" => Category::OursOnly,
            "to-fix" => Category::ToFix,
            other => panic!("unknown category {other}: {line}"),
        };
        let types = match category {
            Category::LaterChange | Category::NotOneToOne => true,
            Category::ToFix => fields.len() == 5,
            _ => false,
        };
        assert_eq!(
            fields.len(),
            if types { 5 } else { 3 },
            "field count: {line}"
        );
        let list = |s: &'static str| -> Vec<&'static str> { s.split(',').collect() };
        entries.push(Allowed {
            category,
            objects: list(fields[1]),
            properties: list(fields[2]),
            ours: if types { fields[3] } else { "" },
            xd: if types { fields[4] } else { "" },
            why: why.trim(),
        });
    }
    entries
}
