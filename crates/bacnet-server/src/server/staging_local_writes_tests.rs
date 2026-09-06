use super::cov_notifications_tests::RecordingTransport;
use super::*;
use bacnet_objects::binary::{BinaryOutputObject, BinaryValueObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::lighting::BinaryLightingOutputObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::Reliability;
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc as StdArc, Mutex as StdMutex};

fn reference(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
    }
}

fn config(
    target_references: Vec<BACnetDeviceObjectReference>,
    first_values: Vec<bool>,
    second_values: Vec<bool>,
) -> StagingConfig {
    StagingConfig {
        present_value: 5.0,
        min_present_value: 0.0,
        units: 62,
        priority_for_writing: 8,
        stages: vec![
            BACnetStageLimitValue {
                limit: 10.0,
                values: first_values,
                deadband: 1.0,
            },
            BACnetStageLimitValue {
                limit: 20.0,
                values: second_values,
                deadband: 1.0,
            },
        ],
        target_references,
        stage_names: None,
    }
}

fn add_device(db: &mut ObjectDatabase) {
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 100,
            name: "Staging test device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
}

async fn read(
    server: &BACnetServer<RecordingTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, array_index)
        .unwrap()
}

#[tokio::test]
async fn staging_writes_bo_bv_blo_at_priority_skips_wildcard_and_notifies_target() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let source = ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap();
    let targets = [
        ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
        ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap(),
        ObjectIdentifier::new(ObjectType::BINARY_LIGHTING_OUTPUT, 1).unwrap(),
    ];
    let mut db = clocked_test_database();
    add_device(&mut db);
    db.add(Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db.add(Box::new(
        BinaryLightingOutputObject::new(1, "BLO-1").unwrap(),
    ))
    .unwrap();
    let references = vec![
        reference(ObjectType::BINARY_OUTPUT, 1),
        reference(ObjectType::BINARY_VALUE, 1),
        reference(ObjectType::BINARY_LIGHTING_OUTPUT, 1),
        reference(ObjectType::BINARY_OUTPUT, ObjectIdentifier::MAX_INSTANCE),
    ];
    db.add(Box::new(
        StagingObject::new(
            1,
            "STG-1",
            config(
                references,
                vec![false, true, false, true],
                vec![true, false, true, false],
            ),
        )
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::generic_builder()
        .transport(RecordingTransport::new(StdArc::clone(&sent)))
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();

    for (target, expected) in targets.iter().copied().zip([0_u32, 1, 0]) {
        assert_eq!(
            read(&server, target, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
            PropertyValue::Enumerated(expected)
        );
    }
    assert_eq!(
        read(&server, source, PropertyIdentifier::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );

    server.cov_table.write().await.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: targets[1],
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_value: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    });
    sent.lock().unwrap().clear();
    server
        .write_local(
            &source,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(15.0),
            None,
        )
        .await
        .unwrap();

    for target in targets {
        assert_eq!(
            read(&server, target, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
            PropertyValue::Enumerated(if target.object_type() == ObjectType::BINARY_VALUE {
                0
            } else {
                1
            })
        );
    }
    assert_eq!(sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn out_of_service_suppresses_targets_and_in_service_reapplies_current_stage() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let source = ObjectIdentifier::new(ObjectType::STAGING, 2).unwrap();
    let target = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 2).unwrap();
    let mut db = clocked_test_database();
    add_device(&mut db);
    db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
        .unwrap();
    db.add(Box::new(
        StagingObject::new(
            2,
            "STG-2",
            config(
                vec![reference(ObjectType::BINARY_VALUE, 2)],
                vec![false],
                vec![true],
            ),
        )
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::generic_builder()
        .transport(RecordingTransport::new(sent))
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();

    server
        .write_local(
            &source,
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .await
        .unwrap();
    server
        .write_local(
            &source,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(15.0),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&server, target, PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Enumerated(0)
    );
    server
        .write_local(
            &source,
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&server, target, PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Enumerated(1)
    );
}

#[tokio::test]
async fn target_failure_faults_source_and_current_success_recovers() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let source = ObjectIdentifier::new(ObjectType::STAGING, 3).unwrap();
    let target = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 9).unwrap();
    let mut db = clocked_test_database();
    add_device(&mut db);
    db.add(Box::new(
        StagingObject::new(
            3,
            "STG-3",
            config(
                vec![reference(ObjectType::BINARY_OUTPUT, 9)],
                vec![false],
                vec![true],
            ),
        )
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::generic_builder()
        .transport(RecordingTransport::new(sent))
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    assert_eq!(
        read(&server, source, PropertyIdentifier::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw())
    );

    server
        .database()
        .write()
        .await
        .add(Box::new(BinaryOutputObject::new(9, "BO-9").unwrap()))
        .unwrap();
    server
        .write_local(
            &source,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(15.0),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&server, target, PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        read(&server, source, PropertyIdentifier::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}

struct CountingBinaryOutput {
    oid: ObjectIdentifier,
    writes: StdArc<AtomicUsize>,
}

impl BACnetObject for CountingBinaryOutput {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "counting-output"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::BINARY_OUTPUT.to_raw(),
            )),
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::Enumerated(0)),
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::PRESENT_VALUE
            && matches!(value, PropertyValue::Enumerated(0 | 1))
            && priority == Some(8)
        {
            self.writes.fetch_add(1, AtomicOrdering::SeqCst);
            return Ok(());
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
        ])
    }
}

#[tokio::test]
async fn retained_stage_does_not_emit_a_duplicate_plan() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let writes = StdArc::new(AtomicUsize::new(0));
    let source = ObjectIdentifier::new(ObjectType::STAGING, 4).unwrap();
    let mut db = clocked_test_database();
    add_device(&mut db);
    db.add(Box::new(CountingBinaryOutput {
        oid: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 4).unwrap(),
        writes: StdArc::clone(&writes),
    }))
    .unwrap();
    db.add(Box::new(
        StagingObject::new(
            4,
            "STG-4",
            config(
                vec![reference(ObjectType::BINARY_OUTPUT, 4)],
                vec![false],
                vec![true],
            ),
        )
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::generic_builder()
        .transport(RecordingTransport::new(sent))
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    assert_eq!(writes.load(AtomicOrdering::SeqCst), 1);

    server
        .write_local(
            &source,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.5),
            None,
        )
        .await
        .unwrap();
    assert_eq!(writes.load(AtomicOrdering::SeqCst), 1);
    server
        .write_local(
            &source,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(11.5),
            None,
        )
        .await
        .unwrap();
    assert_eq!(writes.load(AtomicOrdering::SeqCst), 2);
}
