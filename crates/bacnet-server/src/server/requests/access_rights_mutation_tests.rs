//! Writes of an Access Rights rule array pass the mutation authorizer like
//! any other WriteProperty or WritePropertyMultiple (#1330): the policy sees
//! the write, a denial leaves the rules as they were, and an allowed write
//! lands.

use super::mutation_tests::{Fixture, apdu, assert_denied, oid, wpm};
use super::*;
use crate::mutation::MutationTarget;
use bacnet_objects::access_control::AccessRightsObject;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use std::sync::Mutex as StdMutex;

/// ALWAYS, ALL, disabled.
const ANYWHERE_OFF: &[u8] = &[0x09, 0x01, 0x29, 0x01, 0x49, 0x00];

/// A fixture holding Access Rights 7 with no rules, whose policy records
/// each target it is asked about and answers `allow`.
async fn fixture(allow: bool, seen: Arc<StdMutex<Vec<MutationTarget>>>) -> Fixture {
    let fixture = Fixture::new(Some(Arc::new(move |context| {
        seen.lock().unwrap().push(context.target.clone());
        allow
    })));
    fixture
        .db
        .write()
        .await
        .add(Box::new(AccessRightsObject::new(7, "AR-7").unwrap()))
        .unwrap();
    fixture
}

async fn positive_rule_count(fixture: &Fixture, rights: ObjectIdentifier) -> PropertyValue {
    fixture
        .db
        .read()
        .await
        .get(&rights)
        .unwrap()
        .read_property(PropertyIdentifier::POSITIVE_ACCESS_RULES, Some(0))
        .unwrap()
}

#[tokio::test]
async fn access_rights_rule_writes_pass_the_policy() {
    let rights = oid(ObjectType::ACCESS_RIGHTS, 7);
    let mut write_property = BytesMut::new();
    WritePropertyRequest {
        object_identifier: rights,
        property_identifier: PropertyIdentifier::POSITIVE_ACCESS_RULES,
        property_array_index: None,
        property_value: [ANYWHERE_OFF, ANYWHERE_OFF].concat(),
        priority: None,
    }
    .encode(&mut write_property)
    .unwrap();
    let resize = wpm(vec![WriteAccessSpecification {
        object_identifier: rights,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::POSITIVE_ACCESS_RULES,
            property_array_index: Some(0),
            value: vec![0x21, 0x03],
            priority: None,
        }],
    }]);

    for (service, bytes, written) in [
        (
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_property.freeze(),
            2,
        ),
        (ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, resize, 3),
    ] {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let denied = fixture(false, seen.clone()).await;
        assert_denied(
            denied.dispatch(service, bytes.clone(), 9).await.unwrap(),
            service,
            9,
        );
        let property = match seen.lock().unwrap().as_slice() {
            [MutationTarget::WriteProperty(request)] => request.property_identifier,
            [MutationTarget::WritePropertyMultiple(attempt)] => {
                PropertyIdentifier::from_raw(attempt.reference.property_identifier)
            }
            other => panic!("{service:?}: {other:?}"),
        };
        assert_eq!(property, PropertyIdentifier::POSITIVE_ACCESS_RULES);
        assert_eq!(
            positive_rule_count(&denied, rights).await,
            PropertyValue::Unsigned(0)
        );

        let allowed = fixture(true, Arc::new(StdMutex::new(Vec::new()))).await;
        let reply = allowed.dispatch(service, bytes, 9).await.unwrap();
        assert!(matches!(apdu(reply), Apdu::SimpleAck(_)), "{service:?}");
        assert_eq!(
            positive_rule_count(&allowed, rights).await,
            PropertyValue::Unsigned(written)
        );
    }
}
