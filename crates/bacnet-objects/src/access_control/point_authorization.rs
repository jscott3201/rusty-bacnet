//! The authentication policies, authorization mode and door command priority
//! of an Access Point (Clauses 12.31.10 to 12.31.14 and 12.31.33; #1307,
//! #1325).
//!
//! The application describes the policies; a client picks the one in effect:
//!
//! - Number_Of_Authentication_Policies is 1 until the application sets
//!   another count, which can't be zero (12.31.11). Over the network it is
//!   read-only.
//! - Authentication_Policy_List and Authentication_Policy_Names, the two
//!   optional arrays of Table 12-36, are served once the application sets
//!   them, together, one name per policy. Their size is the policy count
//!   (footnote 1): setting them sets the count, and a new count resizes
//!   them, a new element taking the empty policy with the order not enforced
//!   and no timeout (12.31.12.2) and an empty name. While they are served
//!   the count is at most `MAX_AUTHENTICATION_POLICIES`, so a count can't
//!   make the point allocate without bound. Neither array, nor the count,
//!   takes a network write: Table 12-36 marks none of them W.
//! - Without the list the content of each policy is the application's
//!   (12.31.12), so every policy from 1 to the count counts as usable. With
//!   it, a policy is usable only when its entries are well formed (below);
//!   any other one is invalid.
//! - Active_Authentication_Policy is 1 until changed, and a client changes it
//!   with WriteProperty: an Unsigned naming a usable policy, else
//!   VALUE_OUT_OF_RANGE (12.31.10). Zero names no policy, so it is refused
//!   too.
//! - The active policy drops to zero when the count falls below it or the
//!   list makes it invalid (12.31.10), and stays there until a client writes
//!   a usable one. While it is zero, or the list holds any invalid policy
//!   (12.31.12), the point's Reliability is CONFIGURATION_ERROR
//!   (`AccessPointObject` derives that).
//!
//! A policy's entries are well formed when there is at least one, each names
//! a Credential Data Input object (in a Device, when it names one), and the
//! indexes, in list order, start at 1 and either repeat (another factor that
//! completes the same step) or go up by one. Clause 12.31.12 asks for indexes
//! from 1 in increasing sequence and leaves the rest to the device; this is
//! the reading taken here.
//!
//! Authorization_Mode is writable as well. Table K-10 names it, beside
//! Active_Authentication_Policy, among the Access Point properties an
//! access-control workstation (DS-ACM-A) changes in daily use. Clause
//! 12.31.14 lets a point carry out fewer than all the modes but never leave
//! out AUTHORIZE, so the point keeps the set of modes its application
//! supports. The point enforces no mode itself, so a new point accepts
//! AUTHORIZE alone, the minimum, and an operator's DENY_ALL can't be taken
//! and then ignored by an application that never reads the mode. The
//! application declares the other modes it carries out, standard ones or
//! proprietary ones from 64. A written mode outside the set is
//! VALUE_OUT_OF_RANGE. AUTHORIZE is the mode a new point starts in.
//!
//! Priority_For_Writing is the priority the Access_Doors are commanded at
//! (12.31.32.1), 16 until the application sets another from 1 to 16. As on
//! the Loop it is read-only over the network.
//!
//! The point stores these values and checks them; carrying out a policy or a
//! mode, and commanding the doors, is the application's work.

use bacnet_encoding::constructed::encode_authentication_policy;
use bacnet_types::constructed::{device_identifier_is_device, BACnetAuthenticationPolicy};
use bacnet_types::enums::{AuthorizationMode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::MAX_AUTHENTICATION_POLICIES;
use crate::common;

/// The first proprietary BACnetAuthorizationMode value; 6 to 63 are kept
/// for the standard (Clause 21).
const FIRST_PROPRIETARY_MODE: u32 = 64;

/// One element of Authentication_Policy_List with its
/// Authentication_Policy_Names element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PolicyDefinition {
    name: String,
    policy: BACnetAuthenticationPolicy,
}

/// An Access Point's policy, mode and priority settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Authorization {
    /// Number_Of_Authentication_Policies, at least 1.
    policies: u32,
    /// The two policy arrays, once the application sets them; `policies`
    /// long.
    definitions: Option<Vec<PolicyDefinition>>,
    /// Active_Authentication_Policy: a usable policy, or 0 for none.
    active_policy: u32,
    /// Authorization_Mode, always one of `supported_modes`.
    mode: AuthorizationMode,
    /// The modes Authorization_Mode can take, in numeric order, with
    /// AUTHORIZE always among them.
    supported_modes: Vec<AuthorizationMode>,
    /// Priority_For_Writing, from 1 to 16.
    priority_for_writing: u8,
}

impl Authorization {
    /// One policy, in effect, with no list; AUTHORIZE as the only supported
    /// mode; and the lowest command priority.
    pub(super) fn new() -> Self {
        Self {
            policies: 1,
            definitions: None,
            active_policy: 1,
            mode: AuthorizationMode::AUTHORIZE,
            supported_modes: vec![AuthorizationMode::AUTHORIZE],
            priority_for_writing: 16,
        }
    }

    /// Active_Authentication_Policy: 0 while no usable policy is in effect.
    pub(super) fn active_policy(&self) -> u32 {
        self.active_policy
    }

    /// Whether the point serves the two policy arrays.
    pub(super) fn serves_policy_list(&self) -> bool {
        self.definitions.is_some()
    }

    /// Whether Authentication_Policy_List holds an invalid policy, which
    /// is a configuration error (Clause 12.31.12).
    pub(super) fn holds_invalid_policy(&self) -> bool {
        self.definitions.as_ref().is_some_and(|definitions| {
            definitions
                .iter()
                .any(|definition| !is_well_formed(&definition.policy))
        })
    }

    /// What one of the settings rows reads, or `None` for another property
    /// (or a policy array the point doesn't serve). The two arrays take an
    /// index; the other rows ignore one, which the service handlers refuse
    /// before the object sees it.
    pub(super) fn read(
        &self,
        property: P,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let value = match property {
            P::ACTIVE_AUTHENTICATION_POLICY => PropertyValue::Unsigned(self.active_policy.into()),
            P::NUMBER_OF_AUTHENTICATION_POLICIES => PropertyValue::Unsigned(self.policies.into()),
            P::AUTHORIZATION_MODE => PropertyValue::Enumerated(self.mode.to_raw()),
            P::PRIORITY_FOR_WRITING => PropertyValue::Unsigned(self.priority_for_writing.into()),
            P::AUTHENTICATION_POLICY_LIST => {
                let elements = self
                    .definitions
                    .as_ref()?
                    .iter()
                    .map(|definition| {
                        let mut buf = BytesMut::new();
                        encode_authentication_policy(&mut buf, &definition.policy);
                        PropertyValue::ApplicationData(buf.to_vec())
                    })
                    .collect();
                return Some(common::read_array(elements, array_index));
            }
            P::AUTHENTICATION_POLICY_NAMES => {
                let elements = self
                    .definitions
                    .as_ref()?
                    .iter()
                    .map(|definition| PropertyValue::CharacterString(definition.name.clone()))
                    .collect();
                return Some(common::read_array(elements, array_index));
            }
            _ => return None,
        };
        Some(Ok(value))
    }

    /// A client's write of Active_Authentication_Policy or
    /// Authorization_Mode, or `None` for another property. Neither is an
    /// array, so an index is PROPERTY_IS_NOT_AN_ARRAY; a refusal changes
    /// nothing.
    pub(super) fn write(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != P::ACTIVE_AUTHENTICATION_POLICY && property != P::AUTHORIZATION_MODE {
            return None;
        }
        if array_index.is_some() {
            return Some(Err(common::property_is_not_an_array_error()));
        }
        Some(if property == P::ACTIVE_AUTHENTICATION_POLICY {
            self.write_active_policy(value)
        } else {
            self.write_mode(value)
        })
    }

    /// An Unsigned naming a usable policy, else INVALID_DATA_TYPE or
    /// VALUE_OUT_OF_RANGE.
    fn write_active_policy(&mut self, value: &PropertyValue) -> Result<(), Error> {
        let PropertyValue::Unsigned(policy) = *value else {
            return Err(common::invalid_data_type_error());
        };
        self.active_policy = u32::try_from(policy)
            .ok()
            .filter(|&policy| self.is_usable(policy))
            .ok_or_else(common::value_out_of_range_error)?;
        Ok(())
    }

    /// Whether `policy` names a policy the point can put in effect: one from
    /// 1 to the count, and a valid list element when the list is served.
    fn is_usable(&self, policy: u32) -> bool {
        if !(1..=self.policies).contains(&policy) {
            return false;
        }
        self.definitions.as_ref().is_none_or(|definitions| {
            definitions
                .get(policy as usize - 1)
                .is_some_and(|definition| is_well_formed(&definition.policy))
        })
    }

    /// Drop the active policy to zero when it no longer names a usable one
    /// (Clause 12.31.10).
    fn settle_active_policy(&mut self) {
        if self.active_policy != 0 && !self.is_usable(self.active_policy) {
            self.active_policy = 0;
        }
    }

    /// An Enumerated in the supported set, else INVALID_DATA_TYPE or
    /// VALUE_OUT_OF_RANGE.
    fn write_mode(&mut self, value: &PropertyValue) -> Result<(), Error> {
        let PropertyValue::Enumerated(raw) = *value else {
            return Err(common::invalid_data_type_error());
        };
        let mode = AuthorizationMode::from_raw(raw);
        if !self.supported_modes.contains(&mode) {
            return Err(common::value_out_of_range_error());
        }
        self.mode = mode;
        Ok(())
    }

    /// Replace the policy count: VALUE_OUT_OF_RANGE for zero, or past
    /// [`MAX_AUTHENTICATION_POLICIES`] while the arrays are served. A served
    /// list and its names follow it, and a count below the active policy
    /// drops the active policy to zero.
    pub(super) fn set_policies(&mut self, count: u32) -> Result<(), Error> {
        if count == 0 || (self.definitions.is_some() && count > MAX_AUTHENTICATION_POLICIES) {
            return Err(common::value_out_of_range_error());
        }
        self.policies = count;
        if let Some(definitions) = &mut self.definitions {
            definitions.resize_with(count as usize, PolicyDefinition::default);
        }
        self.settle_active_policy();
        Ok(())
    }

    /// Replace both policy arrays, and with them the count:
    /// VALUE_OUT_OF_RANGE for no policies, or more than
    /// [`MAX_AUTHENTICATION_POLICIES`]. The active policy drops to zero when
    /// the new list leaves it unusable.
    pub(super) fn set_policy_list(
        &mut self,
        policies: Vec<(String, BACnetAuthenticationPolicy)>,
    ) -> Result<(), Error> {
        let count = u32::try_from(policies.len())
            .ok()
            .filter(|count| (1..=MAX_AUTHENTICATION_POLICIES).contains(count))
            .ok_or_else(common::value_out_of_range_error)?;
        self.policies = count;
        self.definitions = Some(
            policies
                .into_iter()
                .map(|(name, policy)| PolicyDefinition { name, policy })
                .collect(),
        );
        self.settle_active_policy();
        Ok(())
    }

    /// Replace the supported modes: VALUE_OUT_OF_RANGE for a mode outside
    /// the production's standard and proprietary values, or a set that
    /// leaves out AUTHORIZE or the mode in effect. Repeats collapse.
    pub(super) fn set_supported_modes(
        &mut self,
        modes: impl IntoIterator<Item = AuthorizationMode>,
    ) -> Result<(), Error> {
        let mut modes: Vec<_> = modes.into_iter().collect();
        if !modes.iter().all(|&mode| in_production(mode))
            || !modes.contains(&AuthorizationMode::AUTHORIZE)
            || !modes.contains(&self.mode)
        {
            return Err(common::value_out_of_range_error());
        }
        modes.sort_unstable_by_key(|mode| mode.to_raw());
        modes.dedup();
        self.supported_modes = modes;
        Ok(())
    }

    /// Replace Priority_For_Writing: VALUE_OUT_OF_RANGE outside 1 to 16.
    pub(super) fn set_priority_for_writing(&mut self, priority: u8) -> Result<(), Error> {
        if !(1..=16).contains(&priority) {
            return Err(common::value_out_of_range_error());
        }
        self.priority_for_writing = priority;
        Ok(())
    }
}

/// Whether `policy`'s entries are well formed (see the module notes): at
/// least one, each naming a Credential Data Input, with indexes from 1 that
/// repeat or go up by one.
fn is_well_formed(policy: &BACnetAuthenticationPolicy) -> bool {
    let mut previous = 0;
    !policy.policy.is_empty()
        && policy.policy.iter().all(|entry| {
            let reference = &entry.credential_data_input;
            let in_sequence =
                entry.index == previous.max(1) || previous.checked_add(1) == Some(entry.index);
            previous = entry.index;
            in_sequence
                && reference.object_identifier.object_type() == ObjectType::CREDENTIAL_DATA_INPUT
                && device_identifier_is_device(reference.device_identifier)
        })
}

/// Whether `mode` is a standard BACnetAuthorizationMode or a proprietary
/// one: 0 to 5, or 64 to 65535.
fn in_production(mode: AuthorizationMode) -> bool {
    let raw = mode.to_raw();
    raw <= AuthorizationMode::NONE.to_raw() || (FIRST_PROPRIETARY_MODE..=65_535).contains(&raw)
}
