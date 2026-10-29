//! Group, GlobalGroup, and StructuredView objects per ASHRAE 135-2020.
//!
//! - GroupObject (type 11) — Clause 12.14
//! - GlobalGroupObject (type 26) — Clause 12.50
//! - StructuredViewObject (type 29) — Clause 12.29

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_encoding::constructed::{
    encode_property_access_result, encode_read_access_specification,
};
use bacnet_types::bitstring::status_flags_from_bacnet;
use bacnet_types::constructed::{
    AccessResult, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetPropertyAccessResult, ReadAccessSpecification,
};
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

use crate::common::{self, read_common_properties, read_identity_properties};
use crate::device_reference;
use crate::traits::BACnetObject;

mod metadata;

// ---------------------------------------------------------------------------
// GroupObject (type 11)
// ---------------------------------------------------------------------------

/// Why [`GroupObject::add_member`] refused a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMemberRefusal {
    /// The specification lists no property references.
    NoProperties,
    /// The specification names a Group or Global Group and selects its
    /// Present_Value, by name or through ALL or REQUIRED.
    NestsGroupPresentValue,
}

impl core::fmt::Display for GroupMemberRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoProperties => f.write_str("the member lists no properties"),
            Self::NestsGroupPresentValue => f.write_str(
                "the member reports a Group or Global Group's Present_Value, \
                 by name or through ALL or REQUIRED",
            ),
        }
    }
}

impl core::error::Error for GroupMemberRefusal {}

impl From<GroupMemberRefusal> for Error {
    fn from(_: GroupMemberRefusal) -> Self {
        common::value_out_of_range_error()
    }
}

/// BACnet Group object (type 11).
///
/// Each member is a [`ReadAccessSpecification`]: an object in this device and
/// the properties of it the group reports (Clause 12.14.5). The specification
/// has no device member, so the Group can't name an object elsewhere; a
/// Global Group does that.
///
/// The object holds no Present_Value. Clause 12.14.6 has it rebuilt from the
/// members on every read, and only the server, which holds the database, can
/// read them: it answers Present_Value with one `ReadAccessResult` per
/// member, read as ReadPropertyMultiple would read that specification. A
/// direct [`read_property`](BACnetObject::read_property) call on the object
/// alone returns an empty list.
pub struct GroupObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    list_of_group_members: Vec<ReadAccessSpecification>,
}

impl GroupObject {
    /// Create a new Group object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::GROUP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            list_of_group_members: Vec::new(),
        })
    }

    /// Append a member to List_Of_Group_Members.
    ///
    /// Refused, leaving the members as they were, with the
    /// [`GroupMemberRefusal`] naming the rule the specification breaks: it
    /// lists no properties, or names a Group or Global Group and selects that
    /// object's Present_Value, explicitly or through ALL or REQUIRED (Clause
    /// 12.14.5 doesn't let one group report another group's Present_Value). A
    /// refusal converts to PROPERTY / VALUE_OUT_OF_RANGE as an [`Error`]. Any
    /// property identifier is taken: ASHRAE assigns some above 4194303 (Table
    /// 23-1), such as Default_Color.
    pub fn add_member(
        &mut self,
        member: ReadAccessSpecification,
    ) -> Result<(), GroupMemberRefusal> {
        let references = &member.list_of_property_references;
        if references.is_empty() {
            return Err(GroupMemberRefusal::NoProperties);
        }
        let nests_a_group = matches!(
            member.object_identifier.object_type(),
            ObjectType::GROUP | ObjectType::GLOBAL_GROUP
        ) && references.iter().any(|reference| {
            matches!(
                reference.property_identifier,
                PropertyIdentifier::PRESENT_VALUE
                    | PropertyIdentifier::ALL
                    | PropertyIdentifier::REQUIRED
            )
        });
        if nests_a_group {
            return Err(GroupMemberRefusal::NestsGroupPresentValue);
        }
        self.list_of_group_members.push(member);
        Ok(())
    }

    /// The members, in List_Of_Group_Members order.
    pub fn members(&self) -> &[ReadAccessSpecification] {
        &self.list_of_group_members
    }

    /// Clear all members from the group.
    pub fn clear_members(&mut self) {
        self.list_of_group_members.clear();
    }
}

impl BACnetObject for GroupObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // Table 12-17 has no Status_Flags, Reliability or Out_Of_Service (#1064).
        if let Some(result) = read_identity_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::GROUP.to_raw()))
            }
            // Both are BACnetLISTs (Table 12-17): an index is refused here as
            // the service handlers refuse it.
            p if (p == PropertyIdentifier::LIST_OF_GROUP_MEMBERS
                || p == PropertyIdentifier::PRESENT_VALUE)
                && array_index.is_some() =>
            {
                Err(common::property_is_not_an_array_error())
            }
            // One ReadAccessSpecification per element (Clause 21).
            p if p == PropertyIdentifier::LIST_OF_GROUP_MEMBERS => Ok(PropertyValue::List(
                self.list_of_group_members
                    .iter()
                    .map(|member| {
                        let mut encoded = BytesMut::new();
                        encode_read_access_specification(&mut encoded, member);
                        PropertyValue::ApplicationData(encoded.to_vec())
                    })
                    .collect(),
            )),
            // Rebuilt by the server on each read; see the type's docs.
            p if p == PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::List(Vec::new())),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            _array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_group_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}

// ---------------------------------------------------------------------------
// GlobalGroupObject (type 26)
// ---------------------------------------------------------------------------

/// BACnet GlobalGroup object (type 26).
///
/// Similar to Group but members are DeviceObjectPropertyReference entries,
/// allowing references to properties on remote devices. GROUP_MEMBER_NAMES
/// provides human-readable names for each member.
///
/// The application sets the members with
/// [`set_group_members`](Self::set_group_members), acquires their values and
/// stores what each read produced in `present_value`, by position in the
/// members: the value, or the error the read failed with. Present_Value goes out as one
/// `BACnetPropertyAccessResult` per member, the member's reference followed
/// by that result (Clause 12.50.7), so the array always has one element per
/// member. A member with no stored result reads as PROPERTY /
/// VALUE_NOT_INITIALIZED, the result Clause 12.50.7.1 gives a new element,
/// and stored results past the last member are not served.
/// Member_Status_Flags is derived from the same store on every read (see
/// [`member_status_flags`](Self::member_status_flags)), so it tracks
/// Present_Value without a second update path. Event_State reads NORMAL: the
/// object has no intrinsic reporting (Clause 12.50.9).
pub struct GlobalGroupObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: Reliability,
    /// The group member references (device, object, property). Only
    /// `set_group_members` changes them, so each Device member is a Device.
    group_members: Vec<BACnetDeviceObjectPropertyReference>,
    /// What the last read of each member produced, by position in
    /// Group_Members (populated externally).
    pub present_value: Vec<AccessResult>,
    /// Human-readable names for each member.
    pub group_member_names: Vec<String>,
}

impl GlobalGroupObject {
    /// Create a new GlobalGroup object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::GLOBAL_GROUP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            group_members: Vec::new(),
            present_value: Vec::new(),
            group_member_names: Vec::new(),
        })
    }

    /// Replace Group_Members, the references whose values the group reports,
    /// each naming an object in this device or, through its device
    /// identifier, in another. The array is read-only over the network.
    ///
    /// A reference whose device identifier isn't a Device object is refused
    /// with PROPERTY / VALUE_OUT_OF_RANGE and the members set before are kept
    /// (#1308). `present_value` and `group_member_names` stay as they are:
    /// they follow the members by position.
    pub fn set_group_members(
        &mut self,
        members: Vec<BACnetDeviceObjectPropertyReference>,
    ) -> Result<(), Error> {
        device_reference::check_device_members(&members)?;
        self.group_members = members;
        Ok(())
    }

    /// Append one reference to Group_Members, refused like
    /// [`set_group_members`](Self::set_group_members): a device identifier
    /// that isn't a Device object is VALUE_OUT_OF_RANGE and adds nothing.
    pub fn add_group_member(
        &mut self,
        member: BACnetDeviceObjectPropertyReference,
    ) -> Result<(), Error> {
        device_reference::check_device_member(member.device_identifier)?;
        self.group_members.push(member);
        Ok(())
    }

    /// The Group_Members references, in order.
    pub fn group_members(&self) -> &[BACnetDeviceObjectPropertyReference] {
        &self.group_members
    }

    /// Member_Status_Flags (Clause 12.50.10): the OR of every Status_Flags
    /// value held in Present_Value.
    ///
    /// A member contributes when its reference names Status_Flags and its
    /// stored result is a bit-string value. Other members, and members with
    /// no stored result, an error result or a value of another type, add
    /// nothing, so a group without Status_Flags members reads all-clear.
    pub fn member_status_flags(&self) -> StatusFlags {
        let status_flags = PropertyIdentifier::STATUS_FLAGS.to_raw();
        self.group_members
            .iter()
            .zip(&self.present_value)
            .filter(|(member, _)| member.property_identifier == status_flags)
            .filter_map(|(_, result)| match result {
                AccessResult::Value(PropertyValue::BitString { data, .. }) => {
                    Some(status_flags_from_bacnet(data))
                }
                _ => None,
            })
            .fold(StatusFlags::empty(), |combined, flags| combined | flags)
    }

    /// The Present_Value elements, one encoded `BACnetPropertyAccessResult`
    /// per member in Group_Members order.
    fn present_value_elements(&self) -> Result<Vec<PropertyValue>, Error> {
        self.group_members
            .iter()
            .enumerate()
            .map(|(index, reference)| {
                let element = BACnetPropertyAccessResult {
                    reference: reference.clone(),
                    access_result: self
                        .present_value
                        .get(index)
                        .cloned()
                        .unwrap_or(AccessResult::NOT_INITIALIZED),
                };
                let mut encoded = BytesMut::new();
                encode_property_access_result(&mut encoded, &element)?;
                Ok(PropertyValue::ApplicationData(encoded.to_vec()))
            })
            .collect()
    }
}

impl BACnetObject for GlobalGroupObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::GLOBAL_GROUP.to_raw()))
            }
            // Each array element is one Clause 21 production: a member
            // reference for Group_Members, a reference and its access result
            // for Present_Value (Table 12-57).
            p if p == PropertyIdentifier::GROUP_MEMBERS => common::read_array(
                device_reference::reference_elements(&self.group_members),
                array_index,
            ),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                common::read_array(self.present_value_elements()?, array_index)
            }
            p if p == PropertyIdentifier::GROUP_MEMBER_NAMES => common::read_array(
                self.group_member_names
                    .iter()
                    .map(|n| PropertyValue::CharacterString(n.clone()))
                    .collect(),
                array_index,
            ),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::NORMAL.to_raw()))
            }
            p if p == PropertyIdentifier::MEMBER_STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.member_status_flags().bits() << 4],
            }),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            _array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_global_group_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}

// ---------------------------------------------------------------------------
// StructuredViewObject (type 29)
// ---------------------------------------------------------------------------

/// BACnet StructuredView object (type 29).
///
/// Provides a hierarchical view of BACnet objects. NODE_TYPE classifies
/// the node role, SUBORDINATE_LIST holds child object references, and
/// SUBORDINATE_ANNOTATIONS provides per-child descriptions. Both are arrays
/// (Table 12-34): index 0 reads the size and each index from 1 one element.
pub struct StructuredViewObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Node type enumeration value (per BACnetNodeType).
    pub node_type: u32,
    /// Node subtype — optional character string.
    pub node_subtype: String,
    /// Child object references, served as BACnetDeviceObjectReference
    /// elements; one without a device names an object in this device. Only
    /// `add_subordinate` adds to it, so each device member is a Device.
    subordinate_list: Vec<BACnetDeviceObjectReference>,
    /// Per-child annotations (parallel to subordinate_list).
    subordinate_annotations: Vec<String>,
}

impl StructuredViewObject {
    /// Create a new StructuredView object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::STRUCTURED_VIEW, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            node_type: 0,
            node_subtype: String::new(),
            subordinate_list: Vec::new(),
            subordinate_annotations: Vec::new(),
        })
    }

    /// Add a subordinate with an annotation. An [`ObjectIdentifier`] alone
    /// names an object in this device.
    ///
    /// A reference whose device identifier isn't a Device object is refused
    /// with VALUE_OUT_OF_RANGE and neither array changes (#1285).
    pub fn add_subordinate(
        &mut self,
        reference: impl Into<BACnetDeviceObjectReference>,
        annotation: impl Into<String>,
    ) -> Result<(), Error> {
        let reference = reference.into();
        device_reference::check_device_member(reference.device_identifier)?;
        self.subordinate_list.push(reference);
        self.subordinate_annotations.push(annotation.into());
        Ok(())
    }

    /// Replace every subordinate, each a reference with its annotation, so
    /// Subordinate_List and Subordinate_Annotations keep one size. An empty
    /// list clears both.
    ///
    /// A reference whose device identifier isn't a Device object is refused
    /// with VALUE_OUT_OF_RANGE and neither array changes (#1285).
    pub fn set_subordinates(
        &mut self,
        subordinates: Vec<(BACnetDeviceObjectReference, String)>,
    ) -> Result<(), Error> {
        for (reference, _) in &subordinates {
            device_reference::check_device_member(reference.device_identifier)?;
        }
        (self.subordinate_list, self.subordinate_annotations) = subordinates.into_iter().unzip();
        Ok(())
    }

    /// The subordinates in order, each reference with its annotation.
    pub fn subordinates(&self) -> impl Iterator<Item = (&BACnetDeviceObjectReference, &str)> {
        self.subordinate_list
            .iter()
            .zip(self.subordinate_annotations.iter().map(String::as_str))
    }
}

impl BACnetObject for StructuredViewObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // Table 12-34 has no Status_Flags, Reliability or Out_Of_Service (#1064).
        if let Some(result) = read_identity_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::STRUCTURED_VIEW.to_raw(),
            )),
            p if p == PropertyIdentifier::NODE_TYPE => {
                Ok(PropertyValue::Enumerated(self.node_type))
            }
            p if p == PropertyIdentifier::NODE_SUBTYPE => {
                Ok(PropertyValue::CharacterString(self.node_subtype.clone()))
            }
            p if p == PropertyIdentifier::SUBORDINATE_LIST => common::read_array(
                device_reference::reference_elements(&self.subordinate_list),
                array_index,
            ),
            p if p == PropertyIdentifier::SUBORDINATE_ANNOTATIONS => common::read_array(
                self.subordinate_annotations
                    .iter()
                    .cloned()
                    .map(PropertyValue::CharacterString)
                    .collect(),
                array_index,
            ),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            _array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_structured_view_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod array_tests;

#[cfg(test)]
mod group_members_tests;

#[cfg(test)]
mod member_status_flags_tests;

#[cfg(test)]
mod tests;
