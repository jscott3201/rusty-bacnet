//! Group, GlobalGroup, and StructuredView objects per ASHRAE 135-2020.
//!
//! - GroupObject (type 11) — Clause 12.14
//! - GlobalGroupObject (type 26) — Clause 12.24
//! - StructuredViewObject (type 29) — Clause 12.29

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// GroupObject (type 11)
// ---------------------------------------------------------------------------

/// BACnet Group object (type 11).
///
/// Groups a set of BACnet objects together. The LIST_OF_GROUP_MEMBERS contains
/// the ObjectIdentifiers of the member objects. PRESENT_VALUE returns the last
/// read results (empty by default).
pub struct GroupObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// The list of group member object identifiers.
    pub list_of_group_members: Vec<ObjectIdentifier>,
    /// The last read results for each member (populated externally).
    pub present_value: Vec<PropertyValue>,
}

impl GroupObject {
    /// Create a new Group object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::GROUP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            list_of_group_members: Vec::new(),
            present_value: Vec::new(),
        })
    }

    /// Add a member object to the group.
    pub fn add_member(&mut self, oid: ObjectIdentifier) {
        self.list_of_group_members.push(oid);
    }

    /// Clear all members from the group.
    pub fn clear_members(&mut self) {
        self.list_of_group_members.clear();
        self.present_value.clear();
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::GROUP.to_raw()))
            }
            p if p == PropertyIdentifier::LIST_OF_GROUP_MEMBERS => Ok(PropertyValue::List(
                self.list_of_group_members
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::List(self.present_value.clone()))
            }
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
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::LIST_OF_GROUP_MEMBERS,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
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
pub struct GlobalGroupObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// The group member references (device, object, property).
    pub group_members: Vec<BACnetDeviceObjectPropertyReference>,
    /// The last read results for each member (populated externally).
    pub present_value: Vec<PropertyValue>,
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
            reliability: 0,
            group_members: Vec::new(),
            present_value: Vec::new(),
            group_member_names: Vec::new(),
        })
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
            p if p == PropertyIdentifier::GROUP_MEMBERS => Ok(PropertyValue::List(
                self.group_members
                    .iter()
                    .map(|r| {
                        PropertyValue::List(vec![
                            PropertyValue::ObjectIdentifier(r.object_identifier),
                            PropertyValue::Unsigned(r.property_identifier as u64),
                            match r.property_array_index {
                                Some(idx) => PropertyValue::Unsigned(idx as u64),
                                None => PropertyValue::Null,
                            },
                            match r.device_identifier {
                                Some(dev) => PropertyValue::ObjectIdentifier(dev),
                                None => PropertyValue::Null,
                            },
                        ])
                    })
                    .collect(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::List(self.present_value.clone()))
            }
            p if p == PropertyIdentifier::GROUP_MEMBER_NAMES => Ok(PropertyValue::List(
                self.group_member_names
                    .iter()
                    .map(|n| PropertyValue::CharacterString(n.clone()))
                    .collect(),
            )),
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
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::GROUP_MEMBERS,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::GROUP_MEMBER_NAMES,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// StructuredViewObject (type 29)
// ---------------------------------------------------------------------------

/// BACnet StructuredView object (type 29).
///
/// Provides a hierarchical view of BACnet objects. NODE_TYPE classifies
/// the node role, SUBORDINATE_LIST holds child object references, and
/// SUBORDINATE_ANNOTATIONS provides per-child descriptions.
pub struct StructuredViewObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// Node type enumeration value (per BACnetNodeType).
    pub node_type: u32,
    /// Node subtype — optional character string.
    pub node_subtype: String,
    /// Child object identifiers.
    pub subordinate_list: Vec<ObjectIdentifier>,
    /// Per-child annotations (parallel to subordinate_list).
    pub subordinate_annotations: Vec<String>,
}

impl StructuredViewObject {
    /// Create a new StructuredView object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::STRUCTURED_VIEW, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            node_type: 0,
            node_subtype: String::new(),
            subordinate_list: Vec::new(),
            subordinate_annotations: Vec::new(),
        })
    }

    /// Add a subordinate object with an annotation.
    pub fn add_subordinate(&mut self, oid: ObjectIdentifier, annotation: impl Into<String>) {
        self.subordinate_list.push(oid);
        self.subordinate_annotations.push(annotation.into());
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
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
            p if p == PropertyIdentifier::SUBORDINATE_LIST => Ok(PropertyValue::List(
                self.subordinate_list
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::SUBORDINATE_ANNOTATIONS => Ok(PropertyValue::List(
                self.subordinate_annotations
                    .iter()
                    .map(|a| PropertyValue::CharacterString(a.clone()))
                    .collect(),
            )),
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
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::NODE_TYPE,
            PropertyIdentifier::NODE_SUBTYPE,
            PropertyIdentifier::SUBORDINATE_LIST,
            PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // GroupObject tests
    // -----------------------------------------------------------------------

    #[test]
    fn group_create() {
        let g = GroupObject::new(1, "Group-1").unwrap();
        assert_eq!(g.object_identifier().object_type(), ObjectType::GROUP);
        assert_eq!(g.object_identifier().instance_number(), 1);
        assert_eq!(g.object_name(), "Group-1");
    }

    #[test]
    fn group_object_type() {
        let g = GroupObject::new(1, "G").unwrap();
        let val = g
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(ObjectType::GROUP.to_raw()));
    }

    #[test]
    fn group_add_members() {
        let mut g = GroupObject::new(1, "G").unwrap();
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
        g.add_member(ai1);
        g.add_member(ai2);

        let val = g
            .read_property(PropertyIdentifier::LIST_OF_GROUP_MEMBERS, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], PropertyValue::ObjectIdentifier(ai1));
            assert_eq!(items[1], PropertyValue::ObjectIdentifier(ai2));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn group_clear_members() {
        let mut g = GroupObject::new(1, "G").unwrap();
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        g.add_member(ai1);
        assert_eq!(g.list_of_group_members.len(), 1);
        g.clear_members();
        assert!(g.list_of_group_members.is_empty());
    }

    #[test]
    fn group_present_value_empty() {
        let g = GroupObject::new(1, "G").unwrap();
        let val = g
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert!(items.is_empty());
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn group_property_list() {
        let g = GroupObject::new(1, "G").unwrap();
        let props = g.property_list();
        assert!(props.contains(&PropertyIdentifier::LIST_OF_GROUP_MEMBERS));
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
    }

    // -----------------------------------------------------------------------
    // GlobalGroupObject tests
    // -----------------------------------------------------------------------

    #[test]
    fn global_group_create() {
        let gg = GlobalGroupObject::new(1, "GG-1").unwrap();
        assert_eq!(
            gg.object_identifier().object_type(),
            ObjectType::GLOBAL_GROUP
        );
        assert_eq!(gg.object_identifier().instance_number(), 1);
        assert_eq!(gg.object_name(), "GG-1");
    }

    #[test]
    fn global_group_object_type() {
        let gg = GlobalGroupObject::new(1, "GG").unwrap();
        let val = gg
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::GLOBAL_GROUP.to_raw())
        );
    }

    #[test]
    fn global_group_members_empty() {
        let gg = GlobalGroupObject::new(1, "GG").unwrap();
        let val = gg
            .read_property(PropertyIdentifier::GROUP_MEMBERS, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert!(items.is_empty());
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn global_group_member_names() {
        let mut gg = GlobalGroupObject::new(1, "GG").unwrap();
        gg.group_member_names.push("Temp Sensor".into());
        gg.group_member_names.push("Humidity".into());

        let val = gg
            .read_property(PropertyIdentifier::GROUP_MEMBER_NAMES, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 2);
            assert_eq!(
                items[0],
                PropertyValue::CharacterString("Temp Sensor".into())
            );
            assert_eq!(items[1], PropertyValue::CharacterString("Humidity".into()));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn global_group_property_list() {
        let gg = GlobalGroupObject::new(1, "GG").unwrap();
        let props = gg.property_list();
        assert!(props.contains(&PropertyIdentifier::GROUP_MEMBERS));
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::GROUP_MEMBER_NAMES));
    }

    // -----------------------------------------------------------------------
    // StructuredViewObject tests
    // -----------------------------------------------------------------------

    #[test]
    fn structured_view_create() {
        let sv = StructuredViewObject::new(1, "SV-1").unwrap();
        assert_eq!(
            sv.object_identifier().object_type(),
            ObjectType::STRUCTURED_VIEW
        );
        assert_eq!(sv.object_identifier().instance_number(), 1);
        assert_eq!(sv.object_name(), "SV-1");
    }

    #[test]
    fn structured_view_object_type() {
        let sv = StructuredViewObject::new(1, "SV").unwrap();
        let val = sv
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::STRUCTURED_VIEW.to_raw())
        );
    }

    #[test]
    fn structured_view_add_subordinates() {
        let mut sv = StructuredViewObject::new(1, "SV").unwrap();
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let bi1 = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap();
        sv.add_subordinate(ai1, "Temperature");
        sv.add_subordinate(bi1, "Occupancy");

        let val = sv
            .read_property(PropertyIdentifier::SUBORDINATE_LIST, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], PropertyValue::ObjectIdentifier(ai1));
            assert_eq!(items[1], PropertyValue::ObjectIdentifier(bi1));
        } else {
            panic!("Expected List");
        }

        let ann = sv
            .read_property(PropertyIdentifier::SUBORDINATE_ANNOTATIONS, None)
            .unwrap();
        if let PropertyValue::List(items) = ann {
            assert_eq!(items.len(), 2);
            assert_eq!(
                items[0],
                PropertyValue::CharacterString("Temperature".into())
            );
            assert_eq!(items[1], PropertyValue::CharacterString("Occupancy".into()));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn structured_view_node_type() {
        let sv = StructuredViewObject::new(1, "SV").unwrap();
        let val = sv
            .read_property(PropertyIdentifier::NODE_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0));
    }

    #[test]
    fn structured_view_node_subtype() {
        let sv = StructuredViewObject::new(1, "SV").unwrap();
        let val = sv
            .read_property(PropertyIdentifier::NODE_SUBTYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString(String::new()));
    }

    #[test]
    fn structured_view_property_list() {
        let sv = StructuredViewObject::new(1, "SV").unwrap();
        let props = sv.property_list();
        assert!(props.contains(&PropertyIdentifier::NODE_TYPE));
        assert!(props.contains(&PropertyIdentifier::NODE_SUBTYPE));
        assert!(props.contains(&PropertyIdentifier::SUBORDINATE_LIST));
        assert!(props.contains(&PropertyIdentifier::SUBORDINATE_ANNOTATIONS));
    }
}
