use super::*;
use bacnet_encoding::constructed::decode_value_source;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_services::wpm::WritePropertyMultipleError;
use bacnet_types::constructed::{BACnetAddress, BACnetValueSource};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::BACnetTimeStamp;
use PropertyIdentifier as P;

fn commanded(kind: Case) -> (PropertyValue, &'static [u8], PropertyValue) {
    match kind.kind {
        ObjectType::ANALOG_OUTPUT => (
            PropertyValue::Real(2.5),
            &[0x44, 0x40, 0x20, 0, 0],
            PropertyValue::Real(0.0),
        ),
        ObjectType::BINARY_OUTPUT => (
            PropertyValue::Enumerated(1),
            &[0x91, 1],
            PropertyValue::Enumerated(0),
        ),
        ObjectType::MULTI_STATE_OUTPUT => (
            PropertyValue::Unsigned(2),
            &[0x21, 2],
            PropertyValue::Unsigned(1),
        ),
        _ => unreachable!(),
    }
}

fn command(kind: Case, invalid: bool) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
        value: if invalid {
            vec![0x11]
        } else {
            commanded(kind).1.to_vec()
        },
        priority: Some(8),
    }
}

fn refused(bytes: Vec<u8>, oid: ObjectIdentifier, property: P, class: ErrorClass, code: ErrorCode) {
    let Apdu::Error(pdu) = bacnet_encoding::apdu::decode_apdu(Bytes::from(bytes)).unwrap() else {
        panic!("expected WPM error")
    };
    assert_eq!(pdu.invoke_id, 5);
    let error = WritePropertyMultipleError::from_error_pdu(&pdu).unwrap();
    assert_eq!(error.error_class, class);
    assert_eq!(error.error_code, code);
    assert_eq!(error.first_failed_write_attempt.object_identifier, oid);
    assert_eq!(
        error.first_failed_write_attempt.property_identifier,
        property.to_raw()
    );
    assert_eq!(error.first_failed_write_attempt.property_array_index, None);
}

async fn command_state(fixture: &Fixture, oid: ObjectIdentifier, kind: Case, accepted: bool) {
    let (value, _, default) = commanded(kind);
    assert_eq!(
        fixture.read(oid, P::PRESENT_VALUE).await,
        if accepted { value.clone() } else { default }
    );
    assert_eq!(
        fixture.read(oid, P::CURRENT_COMMAND_PRIORITY).await,
        if accepted {
            PropertyValue::Unsigned(8)
        } else {
            PropertyValue::Null
        }
    );
    let mut expected = vec![PropertyValue::Null; 16];
    if accepted {
        expected[7] = value;
    }
    assert_eq!(
        fixture.read(oid, P::PRIORITY_ARRAY).await,
        PropertyValue::List(expected)
    );
    let db = fixture.db.read().await;
    let object = db.get(&oid).unwrap();
    for index in [None, Some(8), Some(4)] {
        let property = if index.is_none() {
            P::VALUE_SOURCE
        } else {
            P::VALUE_SOURCE_ARRAY
        };
        let PropertyValue::ApplicationData(bytes) = object.read_property(property, index).unwrap()
        else {
            panic!("source")
        };
        let (source, used) = decode_value_source(&bytes, 0).unwrap();
        assert_eq!(used, bytes.len());
        // Fixture dispatch has an unbound routed writer at network 7, MAC 9.
        assert_eq!(
            source,
            if accepted && index != Some(4) {
                BACnetValueSource::Address(BACnetAddress {
                    network_number: 7,
                    mac_address: MacAddr::from_slice(&[9]),
                })
            } else {
                BACnetValueSource::None
            }
        );
    }
    let PropertyValue::ApplicationData(bytes) =
        object.read_property(P::LAST_COMMAND_TIME, None).unwrap()
    else {
        panic!("timestamp")
    };
    let (time, used) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(time, BACnetTimeStamp::SequenceNumber(u16::from(accepted)));
}

fn storage() -> Arc<Storage> {
    let storage = Arc::new(Storage::default());
    *storage.saved.lock().unwrap() = Some(TagsSnapshot {
        tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
    });
    storage
}

async fn settle(fixture: &Fixture, oid: ObjectIdentifier) {
    let wait = fixture
        .db
        .write()
        .await
        .get_mut(&oid)
        .unwrap()
        .durable_writes_internal()
        .unwrap()
        .settle_forgotten_writes();
    if let Some(wait) = wait {
        tokio::task::spawn_blocking(move || wait.block())
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_commands_and_tags_in_either_order_preserve_separate_owners() {
    for kind in OUTPUTS {
        for tags_first in [false, true] {
            let storage = storage();
            let (fixture, oid) = served_by(kind, &storage).await;
            let mut writes = vec![command(kind, false), attempt(None, FLOOR)];
            if tags_first {
                writes.reverse();
            }
            assert_eq!(
                wire(
                    &fixture,
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                    multiple(oid, writes)
                )
                .await,
                SIMPLE_ACK_WPM
            );
            command_state(&fixture, oid, kind, true).await;
            assert_eq!(read_wire(&fixture, oid, None).await, FLOOR);
            assert_eq!(storage.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
            drop(fixture);
            let (rebuilt, oid) = served_by(kind, &storage).await;
            assert_eq!(read_wire(&rebuilt, oid, None).await, FLOOR);
            // Only Tags is persisted by this attached owner.
            command_state(&rebuilt, oid, kind, false).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_tags_save_refusal_preserves_only_the_preceding_command() {
    for kind in OUTPUTS {
        for tags_first in [false, true] {
            let storage = storage();
            let (fixture, oid) = served_by(kind, &storage).await;
            storage.fail.store(true, Ordering::SeqCst);
            let mut writes = vec![command(kind, false), attempt(None, FLOOR)];
            if tags_first {
                writes.reverse();
            }
            let response = wire(
                &fixture,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                multiple(oid, writes),
            )
            .await;
            refused(
                response,
                oid,
                TAGS,
                ErrorClass::DEVICE,
                ErrorCode::OPERATIONAL_PROBLEM,
            );
            command_state(&fixture, oid, kind, !tags_first).await;
            assert_eq!(read_wire(&fixture, oid, None).await, EXHAUST);
            assert_eq!(storage.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
            drop(fixture);
            let (rebuilt, oid) = served_by(kind, &storage).await;
            assert_eq!(read_wire(&rebuilt, oid, None).await, EXHAUST);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_invalid_command_preserves_consumed_tags_and_corrects_unconsumed_tags() {
    for kind in OUTPUTS {
        for tags_first in [false, true] {
            let storage = storage();
            let (fixture, oid) = served_by(kind, &storage).await;
            let mut writes = vec![command(kind, true), attempt(None, FLOOR)];
            if tags_first {
                writes.reverse();
            }
            let response = wire(
                &fixture,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                multiple(oid, writes),
            )
            .await;
            refused(
                response,
                oid,
                P::PRESENT_VALUE,
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
            );
            settle(&fixture, oid).await;
            command_state(&fixture, oid, kind, false).await;
            let expected = if tags_first { FLOOR } else { EXHAUST };
            assert_eq!(read_wire(&fixture, oid, None).await, expected);
            let count = if tags_first { 1 } else { 2 };
            assert_eq!(storage.attempts.load(Ordering::SeqCst), count);
            assert_eq!(storage.saves.load(Ordering::SeqCst), count);
            drop(fixture);
            let (rebuilt, oid) = served_by(kind, &storage).await;
            assert_eq!(read_wire(&rebuilt, oid, None).await, expected);
        }
    }
}
