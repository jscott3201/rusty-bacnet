use super::*;
use crate::session::bus_port::bus_port;
use bacnet_types::error::RemoteError;

/// A value that is not whole application data: a closing tag with no
/// opening one, alone and after 33 NULLs.
fn broken_values() -> Vec<(Vec<u8>, Option<u8>)> {
    vec![
        (vec![0x3e], None),
        ([vec![0; 33], vec![0x3e]].concat(), None),
    ]
}

/// A priority outside 1 to 16.
fn bad_priorities() -> Vec<(Vec<u8>, Option<u8>)> {
    vec![(vec![0], Some(0)), (vec![0], Some(17))]
}

/// An audited write refused before it is sent leaves nothing: no lease, no
/// request, no record, and no sequence number taken. A value that is not
/// whole application data, a priority outside 1 to 16 and a destination
/// that is not one station are each refused so.
#[tokio::test(start_paused = true)]
async fn source_write_preflight_is_silent_with_a_reporter() {
    preflight(true, [broken_values(), bad_priorities()].concat()).await;
}

/// With no source Audit, a priority outside 1 to 16 is refused the same
/// way.
#[tokio::test(start_paused = true)]
async fn endpoint_write_priority_preflight_is_silent_without_a_reporter() {
    preflight(false, bad_priorities()).await;
}

/// Each of `refused`, a value and priority, is refused with nothing sent;
/// so is each destination that is not one station when the write is
/// audited (`reported`). A write then goes, with the first sequence number.
async fn preflight(reported: bool, refused: Vec<(Vec<u8>, Option<u8>)>) {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut endpoint = if reported {
        session(write_database(false), SessionRole::ClientOnly, &sink)
    } else {
        bare_session(SessionRole::ClientOnly, SessionConfig::default())
    };
    endpoint.start().await.unwrap();
    let client = client_of(&endpoint);
    let unsent = |requests: &mut bacnet_network::layer::ApduReceiver, what: &str| {
        let stray = requests.try_recv().map(|sent| decode_apdu(sent.apdu));
        assert!(stray.is_err(), "{what}: {stray:?}");
    };
    for (value, priority) in refused {
        let what = format!("{} octets, priority {priority:?}", value.len());
        let write = client.write_property(
            peer.local_mac(),
            target(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            priority,
        );
        assert!(write.await.is_err(), "{what}");
        assert_eq!(endpoint.active_leases(), 0);
        unsent(&mut requests, &what);
    }
    let not_one_station = [
        vec![1],
        encode_bip_mac([255; 4], 47808).to_vec(),
        encode_bip_mac([127, 0, 0, 1], 0).to_vec(),
        encode_bip_mac([224, 0, 0, 1], 47808).to_vec(),
    ];
    for mac in not_one_station.into_iter().filter(|_| reported) {
        let write = client.write_property(
            &mac[..],
            target(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0],
            None,
        );
        assert!(write.await.is_err(), "{mac:?}");
        unsent(&mut requests, &format!("{mac:?}"));
    }
    assert!(records.try_recv().is_err());
    // Preflight refusal has not consumed the source's sequence number.
    let task = write_value(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
        vec![0],
        None,
    );
    let envelope = receive(&mut requests).await;
    let (invoke, wp) = write_request(&envelope);
    send(&peer, &envelope.source_mac, write_ack(invoke, &wp)).await;
    assert!(task.await.unwrap().is_ok());
    if reported {
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(
            record.source_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(0))
        );
    } else {
        assert!(records.try_recv().is_err());
    }
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A caller that goes away before its write is admitted leaves nothing: no
/// slot, lease, request or record. One that goes away after loses nothing:
/// the session's task retries the write unchanged and records it once.
#[tokio::test(start_paused = true)]
async fn source_write_before_admission_cancel_and_after_admission_retry_are_bounded() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let client = client_of(&session);
    let db = Arc::clone(session.database.as_ref().unwrap());
    let guard = db.write().await;
    let pending = start_write(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    eventually(WAIT, "a source operation is in use", async || {
        session
            .source_audit
            .as_ref()
            .unwrap()
            .available_operations()
            != 64
    })
    .await;
    pending.abort();
    let _ = pending.await;
    drop(guard);
    assert_eq!(
        session
            .source_audit
            .as_ref()
            .unwrap()
            .available_operations(),
        64
    );
    assert_eq!(session.active_leases(), 0);
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());
    let pending = start_write(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    let (invoke, wp) = write_request(&first);
    pending.abort();
    let _ = pending.await;
    let retry = receive(&mut requests).await;
    assert_eq!(retry.apdu, first.apdu);
    send(&peer, &retry.source_mac, write_ack(invoke, &wp)).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    assert!(
        timeout(Duration::from_millis(150), records.recv())
            .await
            .is_err()
    );
    assert_eq!(session.active_leases(), 0);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn source_write_empty_recipient_list_success_and_empty_scalar_remote_error() {
    use bacnet_objects::{analog::AnalogValueObject, notification_class::NotificationClass};
    use bacnet_server::server::{BACnetServer, ServerConfig};
    use bacnet_types::enums::EngineeringUnits;
    let (sink, mut records) = network().await;
    let mut db = crate::DeviceIdentity::new(456, 42)
        .unwrap()
        .build_database()
        .unwrap();
    db.add(Box::new(NotificationClass::new(1, "empty-list").unwrap()))
        .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(7, "scalar", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let mut remote = BACnetServer::start_clockless(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(456, 42).unwrap(),
            ..Default::default()
        },
        db,
        bus_port(),
    )
    .await
    .unwrap();
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = client_of(&session);
    for (object, property, success) in [
        (
            oid(ObjectType::NOTIFICATION_CLASS, 1),
            PropertyIdentifier::RECIPIENT_LIST,
            true,
        ),
        (target(), PropertyIdentifier::PRESENT_VALUE, false),
    ] {
        let result = client
            .write_property(remote.local_mac(), object, property, None, vec![], None)
            .await;
        assert_eq!(result.is_ok(), success);
        if !success {
            assert!(matches!(
                result,
                Err(Error::Remote(RemoteError::Error { .. }))
            ));
        }
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.target_value, Some(vec![]));
        assert_eq!(record.target_object, Some(object));
        assert!(record.current_value.is_none());
        assert_eq!(record.result.is_none(), success);
    }
    session.stop().await.unwrap();
    remote.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn source_write_wrong_service_error_cannot_finish_or_steal_retry_identity() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let client = client_of(&session);
    let pending = start_write(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    let (invoke, wp) = write_request(&first);
    send(
        &peer,
        &first.source_mac,
        Apdu::Error(ErrorPdu {
            invoke_id: invoke,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            error_data: Bytes::new(),
        }),
    )
    .await;
    let retry = receive(&mut requests).await;
    assert_eq!(retry.apdu, first.apdu);
    assert!(!pending.is_finished());
    assert!(records.try_recv().is_err());
    send(&peer, &retry.source_mac, write_ack(invoke, &wp)).await;
    pending.await.unwrap().unwrap();
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert!(records.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
