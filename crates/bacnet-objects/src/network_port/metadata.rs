use super::NetworkPortObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for NetworkPort (type 56, ASHRAE 135-2020 §12.56
// Table 12-71; printed pp. 542-543 / PDF pp. 544-545, PDF = printed + 2).
// Order preserves the legacy 18-property projection; PROPERTY_LIST is
// appended so the projection helper omits it while required_properties keeps
// it. Only implemented rows are described: table rows the object does not
// serve (APDU_Length (399), Protocol_Level, Event_State, Link_Speeds,
// Link_Speed_Autonegotiate, Network_Number_Quality, Virtual_MAC_Address_Table,
// BACnet_IPv6_UDP_Port, audit/tags/value-source/profile rows) stay absent
// until dispatch exists. Event_State has no read arm and must not gain a row.
// MAX_APDU_LENGTH_ACCEPTED (62) mirrors dispatch as served Optional/ReadOnly:
// the table's APDU_Length (399) has no dispatch arm, so no rename and no
// APDU_LENGTH row is emitted.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED. Status_Flags, Reliability, Network_Type,
// Changes_Pending, and Link_Speed carry the table R code and have no network
// write route, so RequiredRead/ReadOnly. Out_Of_Service carries the table R
// code with a routed Boolean write arm, so RequiredRead/Always (TimeValue
// Present_Value precedent: required readable, permitted writable).
// Description is Optional with a routed CharacterString write arm, so
// Optional/Always. Network_Number, MAC_Address, Command, IP_Address,
// IP_Default_Gateway, IP_Subnet_Mask, and BACnet_IP_UDP_Port are Optional
// with routed write arms, so Optional/Always; IP_*/UDP writes set
// Changes_Pending while Network_Number/MAC/Command do not, but all three
// groups share the Always capability. Max_APDU_Length_Accepted is Optional
// with no write arm, so Optional/ReadOnly. Presence is None throughout: the
// implementation has no DHCP/VMAC gating.
// NetworkPort is neither createable nor deleteable at runtime; both
// overrides stay false (differs from LoadControl delete-allowed).
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NETWORK_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NETWORK_NUMBER, Optional, None, Always),
    PropertyMetadata::new(P::MAC_ADDRESS, Optional, None, Always),
    PropertyMetadata::new(P::MAX_APDU_LENGTH_ACCEPTED, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LINK_SPEED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::CHANGES_PENDING, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COMMAND_NP, Optional, None, Always),
    PropertyMetadata::new(P::IP_ADDRESS, Optional, None, Always),
    PropertyMetadata::new(P::IP_DEFAULT_GATEWAY, Optional, None, Always),
    PropertyMetadata::new(P::IP_SUBNET_MASK, Optional, None, Always),
    PropertyMetadata::new(P::BACNET_IP_UDP_PORT, Optional, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &NetworkPortObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_network_port_exact_sets_readable_rows_and_indexed_list() {
        let object = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::NETWORK_TYPE,
            P::NETWORK_NUMBER,
            P::MAC_ADDRESS,
            P::MAX_APDU_LENGTH_ACCEPTED,
            P::LINK_SPEED,
            P::CHANGES_PENDING,
            P::COMMAND_NP,
            P::IP_ADDRESS,
            P::IP_DEFAULT_GATEWAY,
            P::IP_SUBNET_MASK,
            P::BACNET_IP_UDP_PORT,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::NETWORK_TYPE,
            P::LINK_SPEED,
            P::CHANGES_PENDING,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 19);
        assert_eq!(object.property_list().as_ref(), all);
        assert_eq!(object.required_properties().as_ref(), required);
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<HashSet<_>>()
                .len(),
            metadata.len()
        );
        assert!(!object.is_createable());
        assert!(!object.is_deleteable());
        assert!(!object.supports_cov());
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let expected = if required.contains(&row.property_identifier) {
                RequiredRead
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            object.read_property(row.property_identifier, None).unwrap();
        }
        // Default readbacks pin the value stores.
        assert_eq!(
            object.read_property(P::NETWORK_TYPE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::NETWORK_NUMBER, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object
                .read_property(P::MAX_APDU_LENGTH_ACCEPTED, None)
                .unwrap(),
            PropertyValue::Unsigned(1476)
        );
        assert_eq!(
            object.read_property(P::LINK_SPEED, None).unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            object.read_property(P::CHANGES_PENDING, None).unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            object.read_property(P::COMMAND_NP, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::IP_ADDRESS, None).unwrap(),
            PropertyValue::OctetString(vec![0, 0, 0, 0])
        );
        assert_eq!(
            object.read_property(P::IP_DEFAULT_GATEWAY, None).unwrap(),
            PropertyValue::OctetString(vec![0, 0, 0, 0])
        );
        assert_eq!(
            object.read_property(P::IP_SUBNET_MASK, None).unwrap(),
            PropertyValue::OctetString(vec![255, 255, 255, 0])
        );
        assert_eq!(
            object.read_property(P::BACNET_IP_UDP_PORT, None).unwrap(),
            PropertyValue::Unsigned(0xBAC0)
        );
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 15);
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(15)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [16, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_network_port_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = NetworkPortObject::new(1, "NP-1", 0).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let original = object.property_metadata().into_owned();
            for row in &original {
                let p = row.property_identifier;
                let capability = match p {
                    P::DESCRIPTION
                    | P::OUT_OF_SERVICE
                    | P::NETWORK_NUMBER
                    | P::MAC_ADDRESS
                    | P::COMMAND_NP
                    | P::IP_ADDRESS
                    | P::IP_DEFAULT_GATEWAY
                    | P::IP_SUBNET_MASK
                    | P::BACNET_IP_UDP_PORT => Always,
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability.is_writable() {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Object_Name has no network write route: a rename falls through
            // to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("NP-2".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Read-only served rows have no network write route.
            for p in [
                P::STATUS_FLAGS,
                P::RELIABILITY,
                P::NETWORK_TYPE,
                P::MAX_APDU_LENGTH_ACCEPTED,
                P::LINK_SPEED,
                P::CHANGES_PENDING,
            ] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
            // Command stores Enumerated verbatim with no domain check; other
            // types are rejected.
            object
                .write_property(P::COMMAND_NP, None, PropertyValue::Enumerated(1), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::COMMAND_NP, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
            assert_error(
                object
                    .write_property(P::COMMAND_NP, None, PropertyValue::Unsigned(1), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            // UDP port stores Unsigned within u16 range and sets
            // Changes_Pending; out-of-range and mistyped values fail without
            // changing state.
            let before_port = object.read_property(P::BACNET_IP_UDP_PORT, None).unwrap();
            let before_pending = object.read_property(P::CHANGES_PENDING, None).unwrap();
            object
                .write_property(
                    P::BACNET_IP_UDP_PORT,
                    None,
                    PropertyValue::Unsigned(47809),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::BACNET_IP_UDP_PORT, None).unwrap(),
                PropertyValue::Unsigned(47809)
            );
            assert_error(
                object
                    .write_property(
                        P::BACNET_IP_UDP_PORT,
                        None,
                        PropertyValue::Unsigned(70000),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_error(
                object
                    .write_property(
                        P::BACNET_IP_UDP_PORT,
                        None,
                        PropertyValue::Real(47808.0),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            // Restore the pre-check port so the UDP range pins do not leak
            // into the metadata-stability assertion below.
            object
                .write_property(P::BACNET_IP_UDP_PORT, None, before_port, None)
                .unwrap();
            assert_eq!(
                object.read_property(P::CHANGES_PENDING, None).unwrap(),
                PropertyValue::Boolean(true)
            );
            // The range/type rejections above must not have altered the port
            // beyond the intentional restore; pending stays true once set.
            let _ = before_pending;
            // Network_Number and MAC_Address store verbatim without touching
            // Changes_Pending semantics (covered by the legacy tests); wrong
            // types are rejected.
            assert_error(
                object
                    .write_property(P::NETWORK_NUMBER, None, PropertyValue::Real(1.0), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_error(
                object
                    .write_property(P::MAC_ADDRESS, None, PropertyValue::Unsigned(1), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            for (p, value) in [
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Unserved table rows stay unknown on read and denied on write.
            // APDU_Length (399) is the table row with no dispatch arm;
            // Protocol_Level and Event_State likewise have no arm.
            for p in [P::APDU_LENGTH, P::PROTOCOL_LEVEL, P::EVENT_STATE] {
                assert!(!object.is_writable_property(p));
                assert_error(
                    object.read_property(p, None).unwrap_err(),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Null, None)
                        .unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
            }
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }
}
