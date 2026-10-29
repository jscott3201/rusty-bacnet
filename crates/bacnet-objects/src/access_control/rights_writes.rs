//! Network writes of an Access Rights object's Positive_Access_Rules and
//! Negative_Access_Rules (Clauses 12.34.9 and 12.34.10).
//!
//! Each array takes a whole-array write, a write of one element at a
//! one-based index, and a write of index 0, which resizes it. The server
//! passes the rules on as their raw octets. They are split and decoded with
//! the shared `decode_access_rule`, then checked by the functions the local
//! setters call, so a peer and the application get the same answers. A
//! refused write leaves the array as it was.
//!
//! The errors follow the Schedule and Channel arrays. An element that doesn't
//! open with the time-range specifier's context tag 0 is INVALID_DATA_TYPE,
//! and one that opens right but doesn't decode is INVALID_DATA_ENCODING. A
//! rule the setters turn away is VALUE_OUT_OF_RANGE. A list or a size past
//! [`MAX_ACCESS_RULES`] is NO_SPACE_TO_WRITE_PROPERTY, an index past the end
//! is INVALID_ARRAY_INDEX, and an index-0 value that isn't an Unsigned is
//! INVALID_DATA_TYPE.
//!
//! Index 0 shrinks an array by dropping rules from its end. Growing it adds
//! the rule Clauses 12.34.9.3 and 12.34.10.1 give a new element that comes
//! with no value: both specifiers SPECIFIED, both references unspecified and
//! the enable flag FALSE, so the rule can't match until it is filled in. The
//! clauses fix only the reserved instance of an unspecified reference; this
//! object makes the time range Schedule 4194303's Present_Value and the
//! location Access Point 4194303, both in this device.
//!
//! [`MAX_ACCESS_RULES`]: super::MAX_ACCESS_RULES

use crate::prelude::*;
use bacnet_encoding::constructed::decode_access_rule;
use bacnet_encoding::tags::Tag;
use bacnet_types::constructed::BACnetAccessRule;
use bacnet_types::enums::{AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::rights::{check_access_rule, check_rule_count, checked_rules};
use crate::{common, device_reference};

/// The rule an index-0 write appends when it lengthens an array, its two
/// references in the shared unset form (`device_reference.rs`, #1417).
pub(super) fn grown_rule() -> BACnetAccessRule {
    BACnetAccessRule {
        time_range_specifier: AccessRuleTimeRangeSpecifier::SPECIFIED,
        time_range: Some(device_reference::unset_reference(ObjectType::SCHEDULE)),
        location_specifier: AccessRuleLocationSpecifier::SPECIFIED,
        location: Some(device_reference::unset_identifier(ObjectType::ACCESS_POINT).into()),
        enable: false,
    }
}

/// Whether a tag can begin a `BACnetAccessRule`: its time-range specifier
/// under context tag `[0]`.
fn starts_rule(tag: &Tag) -> bool {
    tag.is_context(0)
}

/// The rules a written value holds, in order, before any check.
fn decode_rules(value: PropertyValue) -> Result<Vec<BACnetAccessRule>, Error> {
    common::decode_elements(&value, starts_rule, decode_access_rule)
}

/// Apply a WriteProperty of one rule array: the whole array with no index,
/// its size at index 0, or the rule at a one-based index.
pub(super) fn write_rules(
    rules: &mut Vec<BACnetAccessRule>,
    array_index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    match array_index {
        None => *rules = checked_rules(decode_rules(value)?)?,
        Some(0) => {
            let PropertyValue::Unsigned(size) = value else {
                return Err(common::invalid_data_type_error());
            };
            let size = usize::try_from(size).unwrap_or(usize::MAX);
            check_rule_count(size)?;
            rules.resize_with(size, grown_rule);
        }
        Some(index) => {
            let slot = usize::try_from(index - 1)
                .ok()
                .filter(|slot| *slot < rules.len())
                .ok_or_else(common::invalid_array_index_error)?;
            let mut decoded = decode_rules(value)?;
            let rule = match decoded.pop() {
                Some(rule) if decoded.is_empty() => rule,
                _ => return Err(common::invalid_data_encoding_error()),
            };
            check_access_rule(&rule)?;
            rules[slot] = rule;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "rights_writes_tests.rs"]
mod tests;
