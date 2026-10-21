//! The interface picker: which IPv4 interface BACnet/IP binds to when `-i`
//! is omitted, as a dialog: a stderr prompt would corrupt the screen in raw
//! mode.

use crate::core::interfaces::Ipv4Interface;

/// A key the picker understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerKey {
    /// Previous entry.
    Up,
    /// Next entry.
    Down,
    /// Choose the highlighted entry.
    Choose,
    /// Choose entry `n` (1-based) directly.
    Number(usize),
}

/// Picker state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Picker {
    /// The interfaces offered.
    pub(crate) items: Vec<Ipv4Interface>,
    /// Highlighted entry.
    pub(crate) selected: usize,
}

impl Picker {
    /// A picker over `items`, which must not be empty.
    pub(crate) fn new(items: Vec<Ipv4Interface>) -> Self {
        Self { items, selected: 0 }
    }

    /// Apply a key; returns the chosen interface, if any.
    pub(crate) fn key(&mut self, key: PickerKey) -> Option<Ipv4Interface> {
        let last = self.items.len().checked_sub(1)?;
        match key {
            PickerKey::Up => self.selected = self.selected.saturating_sub(1),
            PickerKey::Down => self.selected = (self.selected + 1).min(last),
            PickerKey::Choose => return self.items.get(self.selected).cloned(),
            PickerKey::Number(n) if (1..=self.items.len()).contains(&n) => {
                self.selected = n - 1;
                return self.items.get(self.selected).cloned();
            }
            PickerKey::Number(_) => {}
        }
        None
    }
}
