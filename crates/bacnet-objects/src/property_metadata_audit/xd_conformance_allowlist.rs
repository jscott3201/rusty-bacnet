//! Accepted differences between the built-in objects' conformance codes and
//! ASHRAE's XD object-types file, read by `xd_conformance.rs` (#2452).
//!
//! The file is protocol revision 24; this stack implements revision 22
//! (135-2020).
//!
//! One entry per line: `<category> <objects> <properties> [<ours> <file's>] | <reason>`.
//! `<objects>` and `<properties>` are comma-separated lists in our names,
//! kebab-case (a property we don't name by its number). Every object and
//! property pair an entry names must still differ, in the way its category
//! says. A code difference also names both codes (`R`, `W` or `O`). The
//! reason is in our own words.
//!
//! - `later-addition`: the file requires a property that 135-2020 doesn't define on that object type.
//! - `later-change`: the file's code is a later revision's; ours is the 2020 one.
//! - `ours-only`: an object has a row the file doesn't list on its type.
//! - `to-fix`: our row is wrong at revision 22, or a required row is missing; the reason names
//!   the follow-up.

pub(super) const ALLOWED: &str = "
later-change network-port link-speed R O | Table 12-71 gives Link_Speed R
later-change notification-class notification-class R O | Table 12-24 gives Notification_Class R

to-fix load-control status-flags O R | Table 12-32 gives Status_Flags R; follow-up to file: Load Control's Table 12-32 rows
to-fix load-control start-time R W | Table 12-32 gives Start_Time W, and the object has no write route for it; same follow-up
to-fix load-control duty-window,enable,shed-level-descriptions,shed-levels | Table 12-32 requires these, and the object doesn't serve them; same follow-up
";

/// Why a difference is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Category {
    LaterAddition,
    LaterChange,
    OursOnly,
    ToFix,
}

/// One accepted difference, or the same one on several pairs.
#[derive(Debug)]
pub(super) struct Allowed {
    pub category: Category,
    pub objects: Vec<&'static str>,
    pub properties: Vec<&'static str>,
    /// For a code difference: our code and the file's.
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
            "later-change" => Category::LaterChange,
            "ours-only" => Category::OursOnly,
            "to-fix" => Category::ToFix,
            other => panic!("unknown category {other}: {line}"),
        };
        let codes = match category {
            Category::LaterChange => true,
            Category::ToFix => fields.len() == 5,
            _ => false,
        };
        assert_eq!(
            fields.len(),
            if codes { 5 } else { 3 },
            "field count: {line}"
        );
        let list = |s: &'static str| -> Vec<&'static str> { s.split(',').collect() };
        entries.push(Allowed {
            category,
            objects: list(fields[1]),
            properties: list(fields[2]),
            ours: if codes { fields[3] } else { "" },
            xd: if codes { fields[4] } else { "" },
            why: why.trim(),
        });
    }
    entries
}
