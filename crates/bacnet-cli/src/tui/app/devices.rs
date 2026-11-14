//! The device table: rows from I-Am, one per address (#2560), sorting,
//! filtering, selection and the duplicate-instance record.
//!
//! Two devices sharing an instance are two rows, each keyed by where it
//! answers, as the client's discovery table keys them. The duplicate banner
//! is read off those rows: every claimant of an instance is a row, so the
//! table knows each contest without the client's collision notices.
//!
//! Sorting and filtering live in the model. [`DeviceTable::settle`] rebuilds
//! the visible order only when something changed, and the view renders only
//! the rows that fit on screen. The filter and the Name sort use the names
//! already in the session's cache (#1929), and never ask for one.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use bacnet_ops::DeviceAddress;
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

/// The live device table, one row per address.
pub(crate) struct DeviceTable {
    rows: HashMap<DeviceAddress, DeviceRow>,
    /// The addresses claiming each instance.
    by_instance: HashMap<u32, BTreeSet<DeviceAddress>>,
    sort: SortKey,
    descending: bool,
    filter: String,
    visible: Vec<DeviceAddress>,
    selected: Option<DeviceAddress>,
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
            by_instance: HashMap::new(),
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
    /// Insert a row, or refresh the row at its address. Returns the
    /// instance the address claimed before when it claimed another, so the
    /// caller's duplicate record can follow both.
    pub(crate) fn upsert(&mut self, row: DeviceRow) -> Option<u32> {
        let (reorders, before) = match self.rows.get(&row.at) {
            // A refresh changes only `last_seen` unless the route did.
            Some(old) => (
                self.sort == SortKey::LastSeen
                    || old.instance != row.instance
                    || old.address != row.address
                    || old.via != row.via
                    || old.network != row.network
                    || old.vendor_id != row.vendor_id,
                (old.instance != row.instance).then_some(old.instance),
            ),
            None => (true, None),
        };
        if let Some(old) = before {
            self.unclaim(old, &row.at);
        }
        self.by_instance
            .entry(row.instance)
            .or_default()
            .insert(row.at.clone());
        self.rows.insert(row.at.clone(), row);
        self.stale |= reorders;
        before
    }

    /// Remove the row at `row`'s address if it shows `row`'s instance;
    /// true when it did.
    pub(crate) fn remove(&mut self, row: &DeviceRow) -> bool {
        let shown = self
            .rows
            .get(&row.at)
            .is_some_and(|shown| shown.instance == row.instance);
        if shown {
            self.rows.remove(&row.at);
            self.unclaim(row.instance, &row.at);
            self.stale = true;
        }
        shown
    }

    fn unclaim(&mut self, instance: u32, at: &DeviceAddress) {
        if let Some(claims) = self.by_instance.get_mut(&instance) {
            claims.remove(at);
            if claims.is_empty() {
                self.by_instance.remove(&instance);
            }
        }
    }

    /// Replace every row (a resync after dropped events).
    pub(crate) fn replace_all(&mut self, rows: Vec<DeviceRow>) {
        self.rows.clear();
        self.by_instance.clear();
        for row in rows {
            self.upsert(row);
        }
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

    /// The row of the one address claiming `instance`; `None` when none
    /// does, or two or more do and the instance names neither.
    pub(crate) fn get(&self, instance: u32) -> Option<&DeviceRow> {
        let claims = self.by_instance.get(&instance)?;
        if claims.len() != 1 {
            return None;
        }
        self.rows.get(claims.first()?)
    }

    /// The row at `address`.
    pub(crate) fn at(&self, address: &DeviceAddress) -> Option<&DeviceRow> {
        self.rows.get(address)
    }

    /// Every row claiming `instance`, ordered by address.
    pub(crate) fn claims(&self, instance: u32) -> Vec<&DeviceRow> {
        self.by_instance
            .get(&instance)
            .into_iter()
            .flatten()
            .filter_map(|at| self.rows.get(at))
            .collect()
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
            direct(
                order
                    .then(a.instance.cmp(&b.instance))
                    .then_with(|| a.at.cmp(&b.at)),
            )
        });
        self.visible = visible.into_iter().map(|(r, _)| r.at.clone()).collect();
        self.fix_selection();
    }

    fn fix_selection(&mut self) {
        self.selected_index = self
            .selected
            .as_ref()
            .and_then(|sel| self.visible.iter().position(|at| at == sel));
        if self.selected_index.is_none() {
            self.selected_index = (!self.visible.is_empty()).then_some(0);
        }
        self.selected = self.selected_index.map(|i| self.visible[i].clone());
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
        self.rows.get(self.selected.as_ref()?)
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
        self.selected = Some(self.visible[next].clone());
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
        Segmentation::SEGMENTED_BOTH => "both".into(),
        Segmentation::SEGMENTED_TRANSMIT => "transmit".into(),
        Segmentation::SEGMENTED_RECEIVE => "receive".into(),
        Segmentation::NO_SEGMENTATION => "none".into(),
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

/// Addresses shown per duplicated instance; more are counted.
const MAX_ADDRESSES: usize = 4;
/// Duplicated instances tracked. The client's discovery table holds at most
/// 4,096 rows, and every duplicated instance takes two of them.
const MAX_INSTANCES: usize = 2_048;
/// Banner lines before the rest are summarised.
pub(crate) const MAX_BANNER_LINES: usize = 3;

/// The addresses claiming one instance.
struct Claims {
    /// The first [`MAX_ADDRESSES`], in order.
    addresses: Vec<String>,
    /// How many more there are.
    extra: usize,
}

impl Claims {
    fn of(claimants: &[&DeviceRow]) -> Self {
        let mut addresses: Vec<String> = claimants.iter().map(|row| row.located()).collect();
        addresses.sort_unstable();
        let extra = addresses.len().saturating_sub(MAX_ADDRESSES);
        addresses.truncate(MAX_ADDRESSES);
        Self { addresses, extra }
    }

    fn line(&self, instance: u32) -> String {
        let mut joined = match self.addresses.as_slice() {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        };
        if self.extra > 0 {
            joined.push_str(&format!(" (+{} more)", self.extra));
        }
        format!("instance {instance} claimed by {joined}")
    }
}

/// Instances that more than one address claims, read off the device
/// table's rows, which hold every claimant. Bounded, and the banner text is
/// rebuilt only when it changes.
#[derive(Default)]
pub(crate) struct Duplicates {
    by_instance: BTreeMap<u32, Claims>,
    banner: Vec<String>,
}

impl Duplicates {
    /// Note that `claimants` claim `instance` now; fewer than two end the
    /// duplicate.
    pub(crate) fn set(&mut self, instance: u32, claimants: &[&DeviceRow]) {
        if claimants.len() < 2 {
            if self.by_instance.remove(&instance).is_some() {
                self.rebuild_banner();
            }
            return;
        }
        if !self.by_instance.contains_key(&instance) && self.by_instance.len() >= MAX_INSTANCES {
            return;
        }
        self.by_instance.insert(instance, Claims::of(claimants));
        self.rebuild_banner();
    }

    /// Rebuild every duplicate from the device table's rows, after a
    /// snapshot replaced them.
    pub(crate) fn rebuild<'a>(&mut self, rows: impl Iterator<Item = &'a DeviceRow>) {
        let mut by_instance: BTreeMap<u32, Vec<&DeviceRow>> = BTreeMap::new();
        for row in rows {
            by_instance.entry(row.instance).or_default().push(row);
        }
        self.by_instance = by_instance
            .into_iter()
            .filter(|(_, claimants)| claimants.len() >= 2)
            .take(MAX_INSTANCES)
            .map(|(instance, claimants)| (instance, Claims::of(&claimants)))
            .collect();
        self.rebuild_banner();
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
            at: address.parse().unwrap(),
            address: address.into(),
            via: None,
            network: None,
            vendor_id: 1,
            max_apdu: 1476,
            segmentation: Segmentation::NO_SEGMENTATION,
            last_seen: Instant::now(),
        }
    }

    fn claims(instance: u32, addresses: impl IntoIterator<Item = String>) -> Vec<DeviceRow> {
        addresses
            .into_iter()
            .map(|address| claim(instance, &address))
            .collect()
    }

    fn refs(rows: &[DeviceRow]) -> Vec<&DeviceRow> {
        rows.iter().collect()
    }

    #[test]
    fn duplicates_show_four_addresses_and_count_the_rest() {
        let mut duplicates = Duplicates::default();
        let nine = (1..=9).map(|i| format!("10.0.0.{i}:47808"));
        duplicates.set(7, &refs(&claims(7, nine)));
        assert_eq!(
            duplicates.lines(),
            [
                "instance 7 claimed by 10.0.0.1:47808, 10.0.0.2:47808, 10.0.0.3:47808 \
              and 10.0.0.4:47808 (+5 more)"
            ]
        );
        // Each notice lists every claimant, so it replaces the last.
        let two = (1..=2).map(|i| format!("10.0.0.{i}:47808"));
        duplicates.set(7, &refs(&claims(7, two)));
        assert_eq!(
            duplicates.lines(),
            ["instance 7 claimed by 10.0.0.1:47808 and 10.0.0.2:47808"]
        );
    }

    /// A contest down to one claimant, or none, ends its duplicate.
    #[test]
    fn a_contest_down_to_one_claimant_leaves_the_banner() {
        let mut duplicates = Duplicates::default();
        let two = |i: u32| claims(i, ["10.0.0.1:47808".into(), "10.0.0.2:47808".into()]);
        duplicates.set(7, &refs(&two(7)));
        duplicates.set(8, &refs(&two(8)));
        duplicates.set(7, &refs(&claims(7, ["10.0.0.2:47808".into()])));
        assert_eq!(duplicates.banner().len(), 1);
        assert!(duplicates.banner()[0].starts_with(" DUPLICATE instance 8"));
        duplicates.set(8, &[]);
        assert!(duplicates.is_empty());
        assert!(duplicates.banner().is_empty());
    }

    #[test]
    fn the_banner_is_capped_and_summarises_the_rest() {
        let mut duplicates = Duplicates::default();
        for instance in 1..=5 {
            let both = ["10.0.0.1:47808".into(), "10.0.0.2:47808".into()];
            duplicates.set(instance, &refs(&claims(instance, both)));
        }
        let banner = duplicates.banner();
        assert_eq!(banner.len(), MAX_BANNER_LINES);
        assert_eq!(banner[2], " DUPLICATE and 3 more duplicated instances");
        assert!(banner[0].starts_with(" DUPLICATE instance 1 claimed by"));
    }
}
