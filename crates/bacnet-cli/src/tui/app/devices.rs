//! The device table: rows from I-Am, sorting, filtering, selection and the
//! duplicate-instance record.
//!
//! Sorting and filtering live in the model. [`DeviceTable::settle`] rebuilds
//! the visible order only when something changed, and the view renders only
//! the rows that fit on screen. The filter and the Name sort use the names
//! already in the session's cache (#1929), and never ask for one.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use bacnet_types::enums::Segmentation;

use super::names::{NameCache, device_object};
use crate::tui::message::DeviceRow;

/// Columns the table can sort by, in the order `s` cycles through them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortKey {
    /// Device instance.
    Instance,
    /// Address text.
    Address,
    /// Remote network (local devices first).
    Network,
    /// Vendor identifier.
    Vendor,
    /// Most recently seen first.
    LastSeen,
    /// The device's name, those not known yet last either way (#1929).
    Name,
}

impl SortKey {
    fn next(self) -> Self {
        match self {
            Self::Instance => Self::Address,
            Self::Address => Self::Network,
            Self::Network => Self::Vendor,
            Self::Vendor => Self::LastSeen,
            Self::LastSeen => Self::Name,
            Self::Name => Self::Instance,
        }
    }

    /// Column name for the title line.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Name => "name",
            Self::Address => "address",
            Self::Network => "network",
            Self::Vendor => "vendor",
            Self::LastSeen => "last seen",
        }
    }
}

/// Cursor movement in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Movement {
    /// One row up.
    Up,
    /// One row down.
    Down,
    /// One page up.
    PageUp,
    /// One page down.
    PageDown,
    /// First row.
    Home,
    /// Last row.
    End,
}

/// The live device table.
pub(crate) struct DeviceTable {
    rows: HashMap<u32, DeviceRow>,
    sort: SortKey,
    descending: bool,
    filter: String,
    visible: Vec<u32>,
    selected: Option<u32>,
    selected_index: Option<usize>,
    stale: bool,
    /// First visible row, kept by the view between frames.
    pub(crate) offset: Cell<usize>,
    /// Rows that fit on screen in the last frame; PgUp and PgDn move this far.
    pub(crate) page: Cell<usize>,
    /// The last frame drew the table with its Name column, so the names
    /// of the rows it showed are asked for (#1896).
    pub(crate) named: Cell<bool>,
    /// The name cache's [`device_changes`](NameCache::device_changes) when the visible
    /// order was built.
    names_seen: u64,
}

impl Default for DeviceTable {
    fn default() -> Self {
        Self {
            rows: HashMap::new(),
            sort: SortKey::Instance,
            descending: false,
            filter: String::new(),
            visible: Vec::new(),
            selected: None,
            selected_index: None,
            stale: false,
            offset: Cell::new(0),
            page: Cell::new(10),
            named: Cell::new(false),
            names_seen: 0,
        }
    }
}

impl DeviceTable {
    /// Insert or replace a row.
    pub(crate) fn upsert(&mut self, row: DeviceRow) {
        let reorders = match self.rows.get(&row.instance) {
            // A refresh changes only `last_seen` unless the device moved.
            Some(old) => {
                self.sort == SortKey::LastSeen
                    || old.address != row.address
                    || old.via != row.via
                    || old.network != row.network
                    || old.vendor_id != row.vendor_id
            }
            None => true,
        };
        self.rows.insert(row.instance, row);
        self.stale |= reorders;
    }

    /// Remove a row.
    pub(crate) fn remove(&mut self, instance: u32) {
        if self.rows.remove(&instance).is_some() {
            self.stale = true;
        }
    }

    /// Replace every row (a resync after dropped events).
    pub(crate) fn replace_all(&mut self, rows: Vec<DeviceRow>) {
        self.rows = rows.into_iter().map(|r| (r.instance, r)).collect();
        self.stale = true;
    }

    /// Rows held, filtered or not.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no device has answered.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row for an instance.
    pub(crate) fn get(&self, instance: u32) -> Option<&DeviceRow> {
        self.rows.get(&instance)
    }

    /// Every row, filtered or not, in no order.
    pub(crate) fn rows(&self) -> impl Iterator<Item = &DeviceRow> + '_ {
        self.rows.values()
    }

    /// Current sort column and direction.
    pub(crate) fn sort(&self) -> (SortKey, bool) {
        (self.sort, self.descending)
    }

    /// Move to the next sort column, ascending.
    pub(crate) fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.descending = false;
        self.stale = true;
    }

    /// Flip the sort direction.
    pub(crate) fn reverse(&mut self) {
        self.descending = !self.descending;
        self.stale = true;
    }

    /// The filter text.
    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    /// Append to the filter.
    pub(crate) fn push_filter(&mut self, ch: char) {
        self.filter.push(ch);
        self.stale = true;
    }

    /// Remove the filter's last character.
    pub(crate) fn pop_filter(&mut self) {
        self.filter.pop();
        self.stale = true;
    }

    /// Clear the filter.
    pub(crate) fn clear_filter(&mut self) {
        if !self.filter.is_empty() {
            self.filter.clear();
            self.stale = true;
        }
    }

    /// Rebuild the visible order if rows, sort or filter changed, or a
    /// name did while the filter or the sort uses names.
    pub(crate) fn settle(&mut self, names: &NameCache) {
        let by_name = !self.filter.is_empty() || self.sort == SortKey::Name;
        if !self.stale && !(by_name && names.device_changes() != self.names_seen) {
            return;
        }
        self.stale = false;
        self.names_seen = names.device_changes();
        let needle = self.filter.to_lowercase();
        let name = |row: &DeviceRow| {
            let object = device_object(row.instance)?;
            let name = names.known(row.instance, &row.address, object)?;
            Some(name.to_lowercase())
        };
        let mut visible: Vec<(&DeviceRow, Option<String>)> = self
            .rows
            .values()
            .map(|row| (row, if by_name { name(row) } else { None }))
            .filter(|(row, name)| needle.is_empty() || matches(row, name.as_deref(), &needle))
            .collect();
        let (key, descending) = (self.sort, self.descending);
        let direct = |order: std::cmp::Ordering| if descending { order.reverse() } else { order };
        visible.sort_by(|(a, a_name), (b, b_name)| {
            let order = match key {
                SortKey::Instance => a.instance.cmp(&b.instance),
                // Names not known yet go last, whichever the direction.
                SortKey::Name => match (a_name, b_name) {
                    (Some(_), None) => return std::cmp::Ordering::Less,
                    (None, Some(_)) => return std::cmp::Ordering::Greater,
                    _ => a_name.cmp(b_name),
                },
                SortKey::Address => a.address.cmp(&b.address),
                SortKey::Network => a.network.cmp(&b.network),
                SortKey::Vendor => a.vendor_id.cmp(&b.vendor_id),
                SortKey::LastSeen => b.last_seen.cmp(&a.last_seen),
            };
            direct(order.then(a.instance.cmp(&b.instance)))
        });
        self.visible = visible.into_iter().map(|(r, _)| r.instance).collect();
        self.fix_selection();
    }

    fn fix_selection(&mut self) {
        self.selected_index = self
            .selected
            .and_then(|sel| self.visible.iter().position(|&i| i == sel));
        if self.selected_index.is_none() {
            self.selected_index = (!self.visible.is_empty()).then_some(0);
        }
        self.selected = self.selected_index.map(|i| self.visible[i]);
    }

    /// Visible rows in display order. Call [`settle`](Self::settle) first.
    pub(crate) fn visible(&self) -> impl Iterator<Item = &DeviceRow> + '_ {
        self.visible.iter().filter_map(|i| self.rows.get(i))
    }

    /// Number of visible rows.
    pub(crate) fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// Visible rows `start..start + len`.
    pub(crate) fn visible_window(
        &self,
        start: usize,
        len: usize,
    ) -> impl Iterator<Item = &DeviceRow> + '_ {
        self.visible
            .iter()
            .skip(start)
            .take(len)
            .filter_map(|i| self.rows.get(i))
    }

    /// The selected row. Call [`settle`](Self::settle) first.
    pub(crate) fn selected_row(&self) -> Option<&DeviceRow> {
        self.rows.get(&self.selected?)
    }

    /// Index of the selected row among the visible rows.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    /// Move the selection.
    pub(crate) fn move_selection(&mut self, movement: Movement, names: &NameCache) {
        self.settle(names);
        let Some(last) = self.visible.len().checked_sub(1) else {
            return;
        };
        let current = self.selected_index.unwrap_or(0);
        let page = self.page.get().max(1);
        let next = match movement {
            Movement::Up => current.saturating_sub(1),
            Movement::Down => (current + 1).min(last),
            Movement::PageUp => current.saturating_sub(page),
            Movement::PageDown => (current + page).min(last),
            Movement::Home => 0,
            Movement::End => last,
        };
        self.selected_index = Some(next);
        self.selected = Some(self.visible[next]);
    }
}

/// Case-insensitive substring match on the text columns and the cached
/// name (lowercase, like `needle`).
fn matches(row: &DeviceRow, name: Option<&str>, needle: &str) -> bool {
    name.is_some_and(|name| name.contains(needle))
        || row.instance.to_string().contains(needle)
        || row.address.to_lowercase().contains(needle)
        || row
            .via
            .as_ref()
            .is_some_and(|via| via.to_lowercase().contains(needle))
        || row.network.is_some_and(|n| n.to_string().contains(needle))
        || row.vendor_id.to_string().contains(needle)
}

/// Short segmentation label.
pub(crate) fn segmentation_label(segmentation: Segmentation) -> String {
    match segmentation {
        Segmentation::BOTH => "both".into(),
        Segmentation::TRANSMIT => "transmit".into(),
        Segmentation::RECEIVE => "receive".into(),
        Segmentation::NONE => "none".into(),
        other => other.to_raw().to_string(),
    }
}

/// Age as `12s`, `4m` or `3h`.
pub(crate) fn age_label(now: Instant, then: Instant) -> String {
    let secs = now.saturating_duration_since(then).as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3_599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3_600),
    }
}

/// Addresses kept per duplicated instance; more are counted, not stored.
const MAX_ADDRESSES: usize = 4;
/// Extra addresses counted per instance before the count stops growing.
const MAX_EXTRA: usize = 64;
/// Duplicated instances tracked. The client's discovery table holds at most
/// 4,096 devices, and every collision names a row in it.
const MAX_INSTANCES: usize = 4_096;
/// Banner lines before the rest are summarised.
pub(crate) const MAX_BANNER_LINES: usize = 3;

/// The addresses claiming one instance.
#[derive(Default)]
struct Claims {
    addresses: BTreeSet<String>,
    /// Hashes of the addresses past [`MAX_ADDRESSES`], so each is counted once.
    extra: BTreeSet<u64>,
}

impl Claims {
    /// Add an address; true if it was new.
    fn add(&mut self, address: &str) -> bool {
        if self.addresses.contains(address) {
            return false;
        }
        if self.addresses.len() < MAX_ADDRESSES {
            return self.addresses.insert(address.to_string());
        }
        if self.extra.len() >= MAX_EXTRA {
            return false;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(address, &mut hasher);
        self.extra.insert(std::hash::Hasher::finish(&hasher))
    }

    fn line(&self, instance: u32) -> String {
        let list: Vec<&str> = self.addresses.iter().map(String::as_str).collect();
        let mut joined = match list.as_slice() {
            [one] => (*one).to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        };
        match self.extra.len() {
            0 => {}
            MAX_EXTRA => joined.push_str(&format!(" (+{MAX_EXTRA} or more)")),
            n => joined.push_str(&format!(" (+{n} more)")),
        }
        format!("instance {instance} claimed by {joined}")
    }
}

/// Instances that more than one address claims, from the client's collision
/// notices. Bounded, and the banner text is rebuilt only when it changes.
#[derive(Default)]
pub(crate) struct Duplicates {
    by_instance: BTreeMap<u32, Claims>,
    banner: Vec<String>,
}

impl Duplicates {
    /// Record that `retained` and `incoming` claim the same instance.
    /// Returns true if this added an address.
    pub(crate) fn record(&mut self, retained: &DeviceRow, incoming: &DeviceRow) -> bool {
        let instance = retained.instance;
        if !self.by_instance.contains_key(&instance) && self.by_instance.len() >= MAX_INSTANCES {
            return false;
        }
        let claims = self.by_instance.entry(instance).or_default();
        let added = claims.add(&retained.located()) | claims.add(&incoming.located());
        if added {
            self.rebuild_banner();
        }
        added
    }

    /// True when no duplicate has been seen.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.by_instance.is_empty()
    }

    /// One line per duplicated instance, such as
    /// `instance 200 claimed by 10.0.0.1:47808 and 10.0.0.2:47808`.
    #[cfg(test)]
    pub(crate) fn lines(&self) -> Vec<String> {
        self.by_instance
            .iter()
            .map(|(instance, claims)| claims.line(*instance))
            .collect()
    }

    /// The banner: at most [`MAX_BANNER_LINES`] lines, the last summarising
    /// any instances that don't fit.
    pub(crate) fn banner(&self) -> &[String] {
        &self.banner
    }

    fn rebuild_banner(&mut self) {
        let total = self.by_instance.len();
        let shown = if total > MAX_BANNER_LINES {
            MAX_BANNER_LINES - 1
        } else {
            total
        };
        self.banner = self
            .by_instance
            .iter()
            .take(shown)
            .map(|(instance, claims)| format!(" DUPLICATE {}", claims.line(*instance)))
            .collect();
        if total > shown {
            self.banner.push(format!(
                " DUPLICATE and {} more duplicated instances",
                total - shown
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(instance: u32, address: &str) -> DeviceRow {
        DeviceRow {
            instance,
            address: address.into(),
            via: None,
            network: None,
            vendor_id: 1,
            max_apdu: 1476,
            segmentation: Segmentation::NONE,
            last_seen: Instant::now(),
        }
    }

    #[test]
    fn duplicates_keep_four_addresses_and_count_the_rest_once() {
        let mut duplicates = Duplicates::default();
        let retained = claim(7, "10.0.0.1:47808");
        for i in 2..=9 {
            duplicates.record(&retained, &claim(7, &format!("10.0.0.{i}:47808")));
        }
        // A repeat of an extra address is not counted twice.
        assert!(!duplicates.record(&retained, &claim(7, "10.0.0.9:47808")));
        assert_eq!(
            duplicates.lines(),
            [
                "instance 7 claimed by 10.0.0.1:47808, 10.0.0.2:47808, 10.0.0.3:47808 \
              and 10.0.0.4:47808 (+5 more)"
            ]
        );
        for i in 0..200 {
            duplicates.record(&retained, &claim(7, &format!("10.0.1.{i}:47808")));
        }
        assert!(duplicates.lines()[0].ends_with("(+64 or more)"));
    }

    #[test]
    fn the_banner_is_capped_and_summarises_the_rest() {
        let mut duplicates = Duplicates::default();
        for instance in 1..=5 {
            duplicates.record(
                &claim(instance, "10.0.0.1:47808"),
                &claim(instance, "10.0.0.2:47808"),
            );
        }
        let banner = duplicates.banner();
        assert_eq!(banner.len(), MAX_BANNER_LINES);
        assert_eq!(banner[2], " DUPLICATE and 3 more duplicated instances");
        assert!(banner[0].starts_with(" DUPLICATE instance 1 claimed by"));
    }
}
