//! The browser's object list: a device's Object_List grouped by object
//! type, with a filter and a selection that skips the group headings.
//!
//! The filter matches an object's label and, once known, its name
//! (#1929). Only names already in the session's cache count: filtering
//! never asks for one.

use std::cell::Cell;
use std::collections::BTreeMap;

use bacnet_ops::label::object_label;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::devices::Movement;
use super::names::NameLookup;

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListRow {
    /// A type heading with the number of its objects shown.
    Group {
        /// The type.
        object_type: ObjectType,
        /// Objects of it that pass the filter.
        count: usize,
    },
    /// An object.
    Object(ObjectIdentifier),
}

/// The grouped, filtered object list.
#[derive(Default)]
pub(crate) struct ObjectList {
    objects: Vec<ObjectIdentifier>,
    filter: String,
    rows: Vec<ListRow>,
    /// Index into `rows`, always an object row.
    selected: Option<usize>,
    /// First visible row, kept by the view between frames.
    pub(crate) offset: Cell<usize>,
    /// Rows that fit on screen in the last frame.
    pub(crate) page: Cell<usize>,
    /// The last frame drew the list with room for names, so the names of
    /// the rows it showed are asked for (#1896).
    pub(crate) named: Cell<bool>,
    /// The objects or the filter changed since `rows` was built.
    stale: bool,
    /// The cache's [`changes`](super::names::NameCache::changes) when
    /// `rows` was built with names.
    names_seen: u64,
}

impl ObjectList {
    /// Replace the objects, keeping the selection when it is still there.
    /// [`settle`](Self::settle) rebuilds the rows.
    pub(crate) fn set(&mut self, objects: Vec<ObjectIdentifier>) {
        self.objects = objects;
        self.stale = true;
    }

    /// Rebuild the rows when the objects or the filter changed, or when
    /// the filter is on and a name changed.
    pub(crate) fn settle(&mut self, names: &NameLookup<'_>) {
        let renamed = !self.filter.is_empty() && names.changes() != self.names_seen;
        if self.stale || renamed {
            self.stale = false;
            self.names_seen = names.changes();
            self.rebuild(names);
        }
    }

    /// Objects held, filtered or not.
    pub(crate) fn len(&self) -> usize {
        self.objects.len()
    }

    /// Objects that pass the filter.
    pub(crate) fn shown(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, ListRow::Object(_)))
            .count()
    }

    /// The lines to draw.
    pub(crate) fn rows(&self) -> &[ListRow] {
        &self.rows
    }

    /// Index of the selected line.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// The selected object.
    pub(crate) fn selected(&self) -> Option<ObjectIdentifier> {
        match self.rows.get(self.selected?)? {
            ListRow::Object(object) => Some(*object),
            ListRow::Group { .. } => None,
        }
    }

    /// The filter text.
    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    /// Append to the filter; [`settle`](Self::settle) applies it.
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
        self.filter.clear();
        self.stale = true;
    }

    fn rebuild(&mut self, names: &NameLookup<'_>) {
        let keep = self.selected();
        let needle = self.filter.to_lowercase();
        let matches = |object: ObjectIdentifier| {
            object_label(object).contains(&needle)
                || names
                    .name(object)
                    .is_some_and(|name| name.to_lowercase().contains(&needle))
        };
        let mut groups: BTreeMap<u32, Vec<ObjectIdentifier>> = BTreeMap::new();
        for &object in &self.objects {
            if needle.is_empty() || matches(object) {
                groups
                    .entry(object.object_type().to_raw())
                    .or_default()
                    .push(object);
            }
        }
        self.rows.clear();
        for (raw, mut members) in groups {
            members.sort_by_key(|object| object.instance_number());
            self.rows.push(ListRow::Group {
                object_type: ObjectType::from_raw(raw),
                count: members.len(),
            });
            self.rows.extend(members.into_iter().map(ListRow::Object));
        }
        self.selected = keep
            .and_then(|object| {
                self.rows
                    .iter()
                    .position(|row| *row == ListRow::Object(object))
            })
            .or_else(|| self.object_rows().next());
    }

    fn object_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, ListRow::Object(_)))
            .map(|(index, _)| index)
    }

    /// Move the selection among the objects.
    pub(crate) fn move_selection(&mut self, movement: Movement) {
        let objects: Vec<usize> = self.object_rows().collect();
        let Some(last) = objects.len().checked_sub(1) else {
            return;
        };
        let current = self
            .selected
            .and_then(|row| objects.iter().position(|&index| index == row))
            .unwrap_or(0);
        let page = self.page.get().max(1);
        let next = match movement {
            Movement::Up => current.saturating_sub(1),
            Movement::Down => (current + 1).min(last),
            Movement::PageUp => current.saturating_sub(page),
            Movement::PageDown => (current + page).min(last),
            Movement::Home => 0,
            Movement::End => last,
        };
        self.selected = Some(objects[next]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::names::NameCache;
    use super::*;

    fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(object_type, instance).unwrap()
    }

    #[test]
    fn objects_are_grouped_by_type_and_the_selection_skips_headings() {
        let cache = NameCache::default();
        let names = NameLookup::new(&cache, 9, "");
        let mut list = ObjectList::default();
        list.set(vec![
            oid(ObjectType::DEVICE, 9),
            oid(ObjectType::ANALOG_INPUT, 2),
            oid(ObjectType::ANALOG_OUTPUT, 1),
            oid(ObjectType::ANALOG_INPUT, 1),
        ]);
        list.settle(&names);
        assert_eq!(list.rows().len(), 7);
        assert_eq!(list.selected(), Some(oid(ObjectType::ANALOG_INPUT, 1)));
        list.move_selection(Movement::Down);
        list.move_selection(Movement::Down);
        assert_eq!(list.selected(), Some(oid(ObjectType::ANALOG_OUTPUT, 1)));
        list.move_selection(Movement::End);
        assert_eq!(list.selected(), Some(oid(ObjectType::DEVICE, 9)));
        // The filter keeps the selection when it still matches.
        list.push_filter('d');
        list.settle(&names);
        assert_eq!(list.shown(), 1);
        assert_eq!(list.selected(), Some(oid(ObjectType::DEVICE, 9)));
        list.clear_filter();
        list.settle(&names);
        assert_eq!(list.shown(), 4);
    }

    /// The filter matches cached names too, whatever their case, and a
    /// name learned later joins the match on the next settle (#1929).
    #[test]
    fn the_filter_matches_cached_names() {
        let mut cache = NameCache::default();
        let ai = |instance| oid(ObjectType::ANALOG_INPUT, instance);
        cache.learn_name(9, "10.0.0.9:47808", ai(1), "Supply Fan".into());
        let mut list = ObjectList::default();
        list.set(vec![ai(1), ai(2), ai(3)]);
        for ch in "fan".chars() {
            list.push_filter(ch);
        }
        list.settle(&NameLookup::new(&cache, 9, "10.0.0.9:47808"));
        assert_eq!(
            list.rows(),
            [
                ListRow::Group {
                    object_type: ObjectType::ANALOG_INPUT,
                    count: 1
                },
                ListRow::Object(ai(1))
            ]
        );
        // Another device's names don't count.
        list.push_filter(' ');
        list.pop_filter();
        list.settle(&NameLookup::new(&cache, 9, "10.0.9.9:47808"));
        assert_eq!(list.shown(), 0);
        cache.learn_name(9, "10.0.0.9:47808", ai(3), "Exhaust FAN".into());
        list.settle(&NameLookup::new(&cache, 9, "10.0.0.9:47808"));
        assert_eq!(list.shown(), 2);
    }
}
