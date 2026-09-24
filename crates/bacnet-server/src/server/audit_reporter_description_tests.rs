use super::*;
use bacnet_services::{
    common::BACnetPropertyValue,
    wpm::{WriteAccessSpecification, WritePropertyMultipleRequest},
};

#[tokio::test]
async fn target_reporter_description_null_wp_wpm_keeps_one_attempt_and_scalar_errors() {
    let mut fixture = plural(vec![configured(1, Some(vec![]), false)]).await;
    let target = oid(ObjectType::AUDIT_REPORTER, 1);
    let before = fixture
        .server
        .db
        .read()
        .await
        .get(&target)
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    fixture
        .server
        .db
        .write()
        .await
        .get_mut(&target)
        .unwrap()
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();
    settle().await;
    assert!(
        records(&fixture).is_empty(),
        "local NULL is not an actual configuration change"
    );
    for multiple in [false, true] {
        for index in [None, Some(0), Some(1)] {
            let mut encoded = BytesMut::new();
            let service = if multiple {
                WritePropertyMultipleRequest {
                    list_of_write_access_specs: vec![WriteAccessSpecification {
                        object_identifier: target,
                        list_of_properties: vec![BACnetPropertyValue {
                            property_identifier: PropertyIdentifier::DESCRIPTION,
                            property_array_index: index,
                            value: vec![0],
                            priority: None,
                        }],
                    }],
                }
                .encode(&mut encoded)
                .unwrap();
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
            } else {
                WritePropertyRequest {
                    object_identifier: target,
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: index,
                    property_value: vec![0],
                    priority: None,
                }
                .encode(&mut encoded)
                .unwrap();
                ConfirmedServiceChoice::WRITE_PROPERTY
            };
            let response = dispatch(&fixture.server, service, encoded.freeze()).await;
            if index.is_none() {
                assert!(matches!(response, Apdu::SimpleAck(_)));
            } else {
                let Apdu::Error(error) = response else {
                    panic!("expected scalar error")
                };
                assert_eq!(
                    (error.error_class, error.error_code),
                    (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
                );
            }
            settle().await;
            assert_eq!(
                fixture
                    .server
                    .db
                    .read()
                    .await
                    .get(&target)
                    .unwrap()
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                before
            );
        }
    }
    let emitted = records(&fixture);
    // Indexed attempts fail at the existing pre-execution scalar gate and stay
    // silent. Both accepted NULL attempts keep original network provenance.
    assert_eq!(emitted.len(), 2);
    for record in emitted {
        assert_eq!(record.target_object, Some(target));
        assert!(record.invoke_id.is_some());
        assert!(matches!(record.source_device, BACnetRecipient::Address(_)));
        assert_eq!(record.target_value, Some(vec![0]));
        assert_eq!(record.result, None);
    }
    fixture.server.stop().await.unwrap();
}
