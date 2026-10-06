//! Access Credential Authorization_Exemptions (#1331): left out until the
//! application sets it, then served as a list of enumerations and read-only
//! over the network; the setter's refusals.

use bacnet_types::enums::{AuthorizationExemption as Exemption, ErrorClass, ErrorCode};

use super::*;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};

const AE: PropertyIdentifier = PropertyIdentifier::AUTHORIZATION_EXEMPTIONS;

fn assert_property_error<T: std::fmt::Debug>(result: Result<T, Error>, code: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code: got }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(got, code.to_raw() as u32, "expected {code:?}");
        }
        other => panic!("expected PROPERTY / {code:?}, got {other:?}"),
    }
}

/// Whether Authorization_Exemptions is in Property_List and the metadata,
/// and how the metadata describes it.
fn assert_listed(credential: &AccessCredentialObject, listed: bool) {
    assert_eq!(credential.property_list().contains(&AE), listed);
    let metadata = credential.property_metadata();
    let row = metadata.iter().find(|row| row.property_identifier == AE);
    assert_eq!(row.is_some(), listed);
    assert!(!credential.is_writable_property(AE));
    assert!(!credential.required_properties().contains(&AE));
    assert!(credential.is_list_property(AE));
    assert!(!credential.is_array_property(AE));
    if let Some(row) = row {
        assert_eq!(row.conformance, PropertyConformance::Optional);
        assert_eq!(row.write_capability, PropertyWriteCapability::ReadOnly);
        // Just before Property_List.
        let order: Vec<_> = metadata.iter().map(|row| row.property_identifier).collect();
        assert_eq!(
            &order[order.len() - 3..],
            [
                PropertyIdentifier::CREDENTIAL_DISABLE,
                AE,
                PropertyIdentifier::PROPERTY_LIST
            ]
        );
    }
}

fn enumerations(raw: &[u32]) -> PropertyValue {
    PropertyValue::List(raw.iter().map(|&r| PropertyValue::Enumerated(r)).collect())
}

#[test]
fn authorization_exemptions_are_left_out_until_the_application_sets_them() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_eq!(credential.authorization_exemptions(), None);
    assert_listed(&credential, false);
    assert_property_error(
        credential.read_property(AE, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_property_error(
        credential.write_property(AE, None, enumerations(&[2]), None),
        ErrorCode::UNKNOWN_PROPERTY,
    );

    let exemptions = vec![
        Exemption::PASSBACK,
        Exemption::ACCESS_RIGHTS,
        Exemption::AUTHORIZATION_DELAY,
        Exemption::from_raw(64),
        Exemption::from_raw(255),
    ];
    credential
        .set_authorization_exemptions(Some(exemptions.clone()))
        .unwrap();
    assert_eq!(
        credential.authorization_exemptions(),
        Some(exemptions.as_slice())
    );
    assert_listed(&credential, true);
    assert_eq!(
        credential.read_property(AE, None).unwrap(),
        enumerations(&[0, 2, 6, 64, 255])
    );
    // Read-only over the network.
    assert_property_error(
        credential.write_property(AE, None, enumerations(&[]), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );

    // An empty list keeps the row.
    credential
        .set_authorization_exemptions(Some(vec![]))
        .unwrap();
    assert_listed(&credential, true);
    assert_eq!(
        credential.read_property(AE, None).unwrap(),
        enumerations(&[])
    );

    credential.set_authorization_exemptions(None).unwrap();
    assert_listed(&credential, false);
    assert_property_error(
        credential.read_property(AE, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

#[test]
fn authorization_exemptions_outside_the_production_are_refused() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .set_authorization_exemptions(Some(vec![Exemption::LOCKOUT]))
        .unwrap();
    // 7 to 63 are reserved for ASHRAE, and the vendor range stops at 255.
    for bad in [7, 63, 256, 65_535] {
        assert_property_error(
            credential.set_authorization_exemptions(Some(vec![
                Exemption::ACCESS_RIGHTS,
                Exemption::from_raw(bad),
            ])),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(
        credential.authorization_exemptions(),
        Some(&[Exemption::LOCKOUT][..])
    );
}
