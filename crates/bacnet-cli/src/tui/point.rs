//! What the device browser reads, as plain data the worker sends and the
//! app and views show: the device card, the object list, and one object's
//! properties, health and priority array.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

/// Identifies one browse request, so a result for a request the user has
/// moved on from is dropped.
pub(crate) type ReqId = u64;

/// What a browse request reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowseRead {
    /// The device card, then the Object_List.
    Device,
    /// One object's properties.
    Object(ObjectIdentifier),
}

/// One device's part of a name request (#1896): the objects on screen
/// without a name. It replaces what the device was last asked for, so rows
/// scrolled off are no longer read (empty withdraws them all); names
/// already read or being read are not read again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameAsk {
    /// The device.
    pub(crate) device: u32,
    /// Its address as the device table shows it, echoed in the results so
    /// each name is filed under the device it was asked about (#1931).
    pub(crate) address: String,
    /// The Max_APDU_Length_Accepted from its I-Am, which sizes the name
    /// reads' batches (#1930); `None` when the device table lacks it.
    pub(crate) max_apdu: Option<u32>,
    /// The objects, in the order the screen shows them.
    pub(crate) objects: Vec<ObjectIdentifier>,
    /// The device answered again after a timeout: names it didn't answer
    /// for may be read again.
    pub(crate) retry: bool,
}

/// How one name read went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadName {
    /// The object's name.
    Named(String),
    /// The device answered without one (an error, a reject, not a string).
    Refused,
    /// The device didn't answer, or the read was given up with it.
    Unreachable,
}

/// How a property list was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadVia {
    /// One ReadPropertyMultiple.
    Rpm,
    /// ReadProperty per property, after the device refused RPM.
    Rp,
}

/// One property and its value, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PropertyRow {
    /// The property.
    pub(crate) property: PropertyIdentifier,
    /// The value as display text, or the error.
    pub(crate) value: Result<String, String>,
}

/// The device card: labelled values in display order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DeviceCard {
    /// `(label, value)` pairs.
    pub(crate) rows: Vec<(&'static str, String)>,
    /// How the card was read.
    pub(crate) via: Option<ReadVia>,
    /// The device's Object_Name, when it was read.
    pub(crate) name: Option<String>,
}

/// The health columns of Clause 12's common status properties, shown as
/// text badges so they never depend on colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Health {
    /// In alarm: a Status_Flags bit, or an Event_State other than NORMAL.
    pub(crate) alarm: bool,
    /// In fault: a Status_Flags bit, or a Reliability other than
    /// NO_FAULT_DETECTED.
    pub(crate) fault: bool,
    /// Overridden by something local to the device.
    pub(crate) overridden: bool,
    /// Out_Of_Service is TRUE.
    pub(crate) out_of_service: bool,
}

impl Health {
    /// The badges that apply, in a fixed order.
    pub(crate) fn badges(self) -> Vec<&'static str> {
        [
            (self.alarm, "[ALM]"),
            (self.fault, "[FLT]"),
            (self.overridden, "[OVR]"),
            (self.out_of_service, "[OOS]"),
        ]
        .into_iter()
        .filter_map(|(set, badge)| set.then_some(badge))
        .collect()
    }
}

/// Number of command priorities (Clause 19.2).
pub(crate) const PRIORITIES: usize = 16;

/// A commandable object's priority array, Relinquish_Default and
/// Present_Value, already formatted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PriorityView {
    /// Slot `i` holds priority `i + 1`; `None` is relinquished (NULL).
    pub(crate) slots: [Option<String>; PRIORITIES],
    /// Relinquish_Default, when read.
    pub(crate) relinquish_default: Option<String>,
    /// Present_Value, when read.
    pub(crate) present_value: Option<String>,
}

impl PriorityView {
    /// The priority in control: the numerically lowest slot holding a
    /// command (Clause 19.2), as 1 to 16. `None` means every slot is
    /// relinquished and Relinquish_Default applies.
    pub(crate) fn winner(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_some).map(|i| i + 1)
    }
}

/// The standard use of each command priority, paraphrasing Table 19-1:
/// six are assigned, and the rest are left to the site.
pub(crate) fn priority_label(priority: usize) -> &'static str {
    match priority {
        1 => "life safety, manual",
        2 => "life safety, automatic",
        5 => "critical equipment",
        6 => "minimum on/off time",
        8 => "operator, manual",
        _ => "site defined",
    }
}

/// One object as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectView {
    /// The object.
    pub(crate) object: ObjectIdentifier,
    /// Every property read, in the order the device gave them.
    pub(crate) properties: Vec<PropertyRow>,
    /// Status badges.
    pub(crate) health: Health,
    /// For a commandable object, its priority array.
    pub(crate) priority: Option<PriorityView>,
    /// How the properties were read.
    pub(crate) via: ReadVia,
    /// Its Object_Name, when it was read.
    pub(crate) name: Option<String>,
}

impl ObjectView {
    /// The value of `property`, if it was read.
    #[cfg(test)]
    pub(crate) fn value(&self, property: PropertyIdentifier) -> Option<&str> {
        self.properties
            .iter()
            .find(|row| row.property == property)
            .and_then(|row| row.value.as_deref().ok())
    }
}

/// A browse result, tagged by the worker with its request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowseResult {
    /// The device card.
    Card(DeviceCard),
    /// Object_List read element by element: `done` of `total` so far.
    Progress {
        /// Elements read.
        done: u32,
        /// Elements in the list.
        total: u32,
    },
    /// The whole Object_List.
    Objects {
        /// Its entries in device order.
        objects: Vec<ObjectIdentifier>,
        /// Read element by element because the whole list couldn't be.
        indexed: bool,
        /// The element-by-element read stopped early, and why; `objects`
        /// holds the elements before it.
        stopped: Option<String>,
    },
    /// One object's properties.
    Object(Box<ObjectView>),
    /// Object names read for a [`NameAsk`].
    Names {
        /// The device read.
        device: u32,
        /// The address the names were asked under.
        address: String,
        /// Each object and how its read went.
        names: Vec<(ObjectIdentifier, ReadName)>,
    },
    /// The request failed.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_winner_is_the_lowest_commanded_slot() {
        let mut view = PriorityView::default();
        assert_eq!(view.winner(), None, "all relinquished");
        view.slots[15] = Some("1".into());
        view.slots[9] = Some("55".into());
        assert_eq!(view.winner(), Some(10));
        view.slots[7] = Some("20".into());
        assert_eq!(view.winner(), Some(8));
    }

    #[test]
    fn badges_follow_a_fixed_order() {
        let health = Health {
            alarm: false,
            fault: true,
            overridden: false,
            out_of_service: true,
        };
        assert_eq!(health.badges(), ["[FLT]", "[OOS]"]);
        assert!(Health::default().badges().is_empty());
    }
}
