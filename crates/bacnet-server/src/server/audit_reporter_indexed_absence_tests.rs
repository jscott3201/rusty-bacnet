//! The indexed absence gate leaves the real server's Audit owner untouched.
use super::*;
use crate::mutation::MutationTarget;
use bacnet_services::wpm::WritePropertyMultipleError;

async fn completed(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.server.request_tasks.counters().confirmed_active != 0
            || !fixture
                .server
                .notification_transactions
                .delivery_workers_idle()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("request and Audit delivery owners must finish");
}

#[tokio::test]
async fn indexed_absence_served_wpm_audits_only_prefix_and_preserves_source_writes() {
    let mut fixture = server(reporter()).await;
    let target = oid(ObjectType::ANALOG_INPUT, 1);
    let known_source = oid(ObjectType::DEVICE, 30);
    fixture
        .server
        .device_bindings
        .write()
        .await
        .insert_configured(DeviceBinding::local(known_source, SOURCE).unwrap(), |_| {
            false
        })
        .unwrap();
    let absent = PropertyIdentifier::from_raw(5555);
    // Both valid and malformed values fail before the WP observer; the outer
    // WP authorizer's existing precedence is deliberately not changed.
    for input in [vec![0x21, 1], vec![0x41, 0]] {
        let mut bytes = BytesMut::new();
        WritePropertyRequest {
            object_identifier: target,
            property_identifier: absent,
            property_array_index: Some(1),
            property_value: input,
            priority: None,
        }
        .encode(&mut bytes)
        .unwrap();
        let Apdu::Error(error) = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            bytes.freeze(),
        )
        .await
        else {
            panic!("expected WP error");
        };
        assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
        );
        completed(&fixture).await;
        assert!(notifications(&fixture.transport.sent).is_empty());
    }
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let seen = calls.clone();
    fixture.server.config_mut().mutation_authorizer = Some(Arc::new(move |context| {
        let MutationTarget::WritePropertyMultiple(attempt) = &context.target else {
            panic!("expected WPM");
        };
        seen.lock().unwrap().push(attempt.clone());
        true
    }));
    let prefix = vec![0x75, 7, 0, b'p', b'r', b'e', b'f', b'i', b'x'];
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: target,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: prefix.clone(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: absent,
                    property_array_index: Some(1),
                    value: vec![0x41, 0],
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: vec![0x72, 0, b'x'],
                    priority: None,
                },
            ],
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    let Apdu::Error(error) = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        bytes.freeze(),
    )
    .await
    else {
        panic!("expected formal WPM failure");
    };
    let detail = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
    assert_eq!(
        (detail.error_class, detail.error_code),
        (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
    );
    assert_eq!(detail.first_failed_write_attempt.object_identifier, target);
    assert_eq!(
        detail.first_failed_write_attempt.property_identifier,
        absent.to_raw()
    );
    assert_eq!(
        detail.first_failed_write_attempt.property_array_index,
        Some(1)
    );
    completed(&fixture).await;
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].reference.object_identifier, target);
        assert_eq!(
            calls[0].reference.property_identifier,
            PropertyIdentifier::DESCRIPTION.to_raw()
        );
        assert_eq!(calls[0].reference.property_array_index, None);
        assert_eq!(calls[0].value, prefix);
    }
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
        PropertyValue::CharacterString("prefix".into())
    );
    let emitted = notifications(&fixture.transport.sent);
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].notifications.len(), 1);
    let record = &emitted[0].notifications[0];
    assert_eq!(record.target_object, Some(target));
    assert_eq!(
        record.target_property,
        Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None
        })
    );
    assert_eq!(record.target_value, Some(prefix));
    assert_eq!(record.invoke_id, Some(error.invoke_id));
    assert_eq!(record.source_device, BACnetRecipient::Device(known_source));
    assert_eq!(record.result, None);
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 0);

    // A served command still reaches the existing source-aware writer and
    // actual Audit delivery, using the same configured source binding.
    fixture.server.config_mut().mutation_authorizer = None;
    let controlled = oid(ObjectType::BINARY_VALUE, 1);
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                controlled,
                PropertyIdentifier::PRESENT_VALUE,
                vec![0x91, 1],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    completed(&fixture).await;
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    let emitted = notifications(&fixture.transport.sent);
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted[1].notifications.len(), 1);
    assert_eq!(emitted[1].notifications[0].target_object, Some(controlled));
    assert_eq!(
        emitted[1].notifications[0].source_device,
        BACnetRecipient::Device(known_source)
    );
    assert_eq!(
        emitted[1].notifications[0].target_value,
        Some(vec![0x91, 1])
    );
    assert_eq!(emitted[1].notifications[0].result, None);
    fixture.server.stop().await.unwrap();
}

/// `write_local` refuses an index on a property that isn't an array before
/// the Audit owner sees the write, as WriteProperty does (#1426).
#[tokio::test]
async fn local_index_refusal_reaches_no_audit_record() {
    let mut fixture = server(reporter()).await;
    let target = oid(ObjectType::ANALOG_INPUT, 1);
    let write = |index| {
        fixture.server.write_local(
            &target,
            PropertyIdentifier::DESCRIPTION,
            index,
            PropertyValue::CharacterString("local".into()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
    };
    let error = write(Some(1)).await.unwrap_err();
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32),
        "{error:?}"
    );
    completed(&fixture).await;
    settle().await;
    assert!(notifications(&fixture.transport.sent).is_empty());

    // The same write without the index is audited.
    write(None).await.unwrap();
    completed(&fixture).await;
    settle().await;
    let emitted = notifications(&fixture.transport.sent);
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].notifications[0].target_object, Some(target));
    fixture.server.stop().await.unwrap();
}

#[path = "audit_reporter_wpm_ahead_tests.rs"]
mod wpm_ahead;
