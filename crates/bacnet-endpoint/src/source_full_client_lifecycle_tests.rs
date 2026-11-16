//! What the full client's source Audit holds and lets go (#2617): a stop
//! cuts an operation off and records nothing, a caller that gives up
//! before admission sends nothing, requests the Reporter doesn't cover go
//! as they are, and a segmented request takes the same path.
use super::*;
use bacnet_encoding::apdu::{SegmentAck, UnconfirmedRequest};
use bacnet_services::file::FileWriteAccessMethod;
use bacnet_services::text_message::TextMessageRequest;
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::{MessagePriority, Segmentation};

/// An audited read waiting for its answer when the session stops gets an
/// error, frees its invoke ID and records nothing. So does one whose
/// caller went away, cut off by the client's own stop or drop.
#[tokio::test(start_paused = true)]
async fn an_audited_operation_cut_off_by_a_stop_is_not_recorded() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    for cut in ["session stop", "client stop", "client drop"] {
        let mut session = session(database(false), SessionRole::Both, &sink);
        session.client_config.apdu_timeout_ms = 60_000;
        session.start().await.unwrap();
        let client = full_client(&session);
        let read = full_read(
            &client,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let request = receive(&mut requests).await;
        if cut == "session stop" {
            session.stop().await.unwrap();
            assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
            drop(client);
        } else {
            // The caller goes away; the session's task keeps the read until
            // the client stops.
            read.abort();
            let _ = read.await;
            let Ok(mut client) = Arc::try_unwrap(client) else {
                panic!("the read's task let its client go")
            };
            if cut == "client stop" {
                client.stop().await.unwrap();
                // A stopped client's next request is refused at once, as
                // on any closed seat, before the session admits it: it
                // takes no slot.
                let next = client.read_property(
                    peer.local_mac(),
                    target(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                );
                let error = Box::pin(next).await.unwrap_err();
                assert!(error.to_string().contains("seat has closed"), "{error}");
                let source = Arc::clone(session.source_audit.as_ref().unwrap());
                eventually(WAIT, "every slot is free", async || {
                    source.available_operations() == 64
                })
                .await;
            }
            drop(client);
        }
        let (invoke, rp) = read_request(&request);
        send(&peer, &request.source_mac, ack(invoke, &rp)).await;
        eventually(WAIT, "invoke IDs are free", async || {
            session.coordinator.active_count().unwrap() == 0
        })
        .await;
        assert!(
            timeout(Duration::from_millis(100), records.recv())
                .await
                .is_err(),
            "{cut}"
        );
        if cut != "session stop" {
            session.stop().await.unwrap();
        }
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A caller that gives up while its read waits to be admitted sends
/// nothing and records nothing, and the admission it held comes back.
#[tokio::test(start_paused = true)]
async fn a_caller_that_gives_up_before_admission_sends_nothing() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let db = Arc::clone(session.database.as_ref().unwrap());
    let guard = db.write().await;
    let read = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    eventually(WAIT, "the read holds an admission", async || {
        source.available_operations() != 64
    })
    .await;
    read.abort();
    let _ = read.await;
    drop(guard);
    assert_eq!(source.available_operations(), 64);
    assert_eq!(session.active_leases(), 0);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// Two audited reads at once: each request's copy of the client ends its
/// own transaction only, so the one answered second still completes.
#[tokio::test(start_paused = true)]
async fn audited_reads_at_once_each_end_only_their_own_transaction() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let first = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first_request = receive(&mut requests).await;
    let second = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    let second_request = receive(&mut requests).await;
    for (request, read) in [(second_request, second), (first_request, first)] {
        let (invoke, rp) = read_request(&request);
        send(&peer, &request.source_mac, ack(invoke, &rp)).await;
        assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.invoke_id, Some(invoke));
        assert_eq!(record.result, None);
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// What the Reporter doesn't cover goes as it is: a read when it records
/// only writes, and a subscription, which this release doesn't audit.
#[tokio::test(start_paused = true)]
async fn requests_the_reporter_does_not_cover_go_unrecorded() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut db = database(false);
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(0b10).unwrap(),
            false,
            None,
            BACnetPriorityFilter::all(),
            None,
        )
        .unwrap();
    let mut session = session(db, SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let read = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    let subscriber = Arc::clone(&client);
    let mac = peer.local_mac().to_vec();
    let subscribe = tokio::spawn(async move {
        subscriber
            .subscribe_cov(&mac[..], 9, target(), false, Some(60))
            .await
    });
    let request = receive(&mut requests).await;
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(request.apdu.clone()).unwrap() else {
        panic!("SubscribeCOV")
    };
    assert_eq!(pdu.service_choice, ConfirmedServiceChoice::SUBSCRIBE_COV);
    let answer = Apdu::SimpleAck(SimpleAck {
        invoke_id: pdu.invoke_id,
        service_choice: pdu.service_choice,
    });
    send(&peer, &request.source_mac, answer).await;
    assert!(timeout(WAIT, subscribe).await.unwrap().unwrap().is_ok());
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(records.try_recv().is_err());
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// An audited request still goes to one station: a B/IP MAC with port 0
/// is refused before anything is sent, and so is a request of an audited
/// service that doesn't decode, since what it names can't be reported.
#[tokio::test(start_paused = true)]
async fn an_audited_request_to_no_one_station_or_that_does_not_decode_is_refused() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let nowhere = encode_bip_mac([127, 0, 0, 1], 0);
    let read = full_read(&client, &nowhere, PropertyIdentifier::PRESENT_VALUE, None);
    let error = timeout(WAIT, read).await.unwrap().unwrap().unwrap_err();
    assert!(error.to_string().contains("unicast"), "{error}");
    let error = Box::pin(client.confirmed_request(
        peer.local_mac(),
        ConfirmedServiceChoice::READ_PROPERTY,
        &[0x0C],
    ))
    .await
    .unwrap_err();
    assert!(error.to_string().contains("must decode"), "{error}");
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());
    assert_eq!(session.active_leases(), 0);
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A write too long for the device goes segmented, by the same path: the
/// client learns from the device's I-Am that it takes 128 octets and
/// segments, sends the file write in segments, and records it once.
#[tokio::test(start_paused = true)]
async fn a_segmented_request_is_audited_as_any_other() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut db = database(false);
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(0b10).unwrap(),
            false,
            None,
            BACnetPriorityFilter::all(),
            None,
        )
        .unwrap();
    let mut session = session(db, SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let mut i_am = BytesMut::new();
    IAmRequest {
        object_identifier: oid(ObjectType::DEVICE, 77),
        max_apdu_length: 128,
        segmentation_supported: Segmentation::SEGMENTED_BOTH,
        vendor_id: 42,
    }
    .encode(&mut i_am);
    send(
        &peer,
        client.local_mac(),
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: i_am.freeze(),
        }),
    )
    .await;
    let watched = Arc::clone(&client);
    eventually(WAIT, "the device is known", async || {
        !watched.discovered_devices().await.is_empty()
    })
    .await;
    let file = oid(ObjectType::FILE, 3);
    let writer = Arc::clone(&client);
    let mac = peer.local_mac().to_vec();
    let write = tokio::spawn(async move {
        let access = FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![7; 400],
        };
        writer.atomic_write_file(&mac[..], file, access).await
    });
    let mut segments = 0;
    let invoke = loop {
        let request = receive(&mut requests).await;
        let Apdu::ConfirmedRequest(pdu) = decode_apdu(request.apdu.clone()).unwrap() else {
            panic!("a segment")
        };
        assert!(pdu.segmented);
        segments += 1;
        let answer = if pdu.more_follows {
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id: pdu.invoke_id,
                sequence_number: pdu.sequence_number.unwrap(),
                actual_window_size: 1,
            })
        } else {
            let mut ack = BytesMut::new();
            bacnet_encoding::primitives::encode_ctx_signed(&mut ack, 0, 0);
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: pdu.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
                service_ack: ack.freeze(),
            })
        };
        send(&peer, &request.source_mac, answer).await;
        if !pdu.more_follows {
            break pdu.invoke_id;
        }
    };
    assert!(segments > 1);
    assert!(timeout(WAIT, write).await.unwrap().unwrap().is_ok());
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.target_object, Some(file));
    assert!(record.target_property.is_none() && record.target_value.is_none());
    assert_eq!(record.result, None);
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A confirmed service outside the ten audited goes as it is on an audited
/// client and records nothing, raw or typed; its caller going away
/// mid-request sends no retry and frees its invoke ID, as on any client;
/// and an unconfirmed send goes out.
#[tokio::test(start_paused = true)]
async fn an_unaudited_service_and_an_unconfirmed_send_go_as_they_are() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_retries = 2;
    session.client_config.apdu_timeout_ms = 3_000;
    session.start().await.unwrap();
    let client = full_client(&session);
    // DeviceCommunicationControl, ENABLE for no set time.
    let communication_control = |client: &Client| {
        let client = Arc::clone(client);
        let mac = peer.local_mac().to_vec();
        tokio::spawn(async move {
            client
                .confirmed_request(
                    &mac[..],
                    ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                    &[0x19, 0x00],
                )
                .await
        })
    };
    let call = communication_control(&client);
    let request = receive(&mut requests).await;
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(request.apdu.clone()).unwrap() else {
        panic!("a confirmed request")
    };
    let answer = Apdu::SimpleAck(SimpleAck {
        invoke_id: pdu.invoke_id,
        service_choice: pdu.service_choice,
    });
    send(&peer, &request.source_mac, answer).await;
    assert!(timeout(WAIT, call).await.unwrap().unwrap().is_ok());

    let call = communication_control(&client);
    let _first = receive(&mut requests).await;
    call.abort();
    let _ = call.await;
    eventually(WAIT, "invoke IDs are free", async || {
        session.coordinator.active_count().unwrap() == 0
    })
    .await;
    assert!(
        timeout(Duration::from_secs(20), requests.recv())
            .await
            .is_err(),
        "no retry after the caller left"
    );

    // A typed call of a service the node doesn't audit goes too.
    let text = {
        let client = Arc::clone(&client);
        let mac = peer.local_mac().to_vec();
        tokio::spawn(async move {
            let message = TextMessageRequest {
                source_device: oid(ObjectType::DEVICE, 1),
                message_class: None,
                message_priority: MessagePriority::NORMAL,
                message: "check pump".into(),
            };
            client.confirmed_text_message(&mac[..], &message).await
        })
    };
    let request = receive(&mut requests).await;
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(request.apdu.clone()).unwrap() else {
        panic!("a confirmed request")
    };
    assert_eq!(
        pdu.service_choice,
        ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE
    );
    let answer = Apdu::SimpleAck(SimpleAck {
        invoke_id: pdu.invoke_id,
        service_choice: pdu.service_choice,
    });
    send(&peer, &request.source_mac, answer).await;
    assert!(timeout(WAIT, text).await.unwrap().unwrap().is_ok());

    client
        .unconfirmed_request(peer.local_mac(), UnconfirmedServiceChoice::WHO_IS, &[])
        .await
        .unwrap();
    let who_is = receive(&mut requests).await;
    assert!(matches!(
        decode_apdu(who_is.apdu.clone()).unwrap(),
        Apdu::UnconfirmedRequest(_)
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(records.try_recv().is_err());
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A client dropped with an audited request in flight, then a new client
/// on the same session: the first request is not recorded, the new
/// client's is, and every invoke ID comes back.
#[tokio::test(start_paused = true)]
async fn a_new_client_after_one_dropped_mid_request_is_audited_alone() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = full_client(&session);
    let read = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    read.abort();
    let _ = read.await;
    let Ok(client) = Arc::try_unwrap(client) else {
        panic!("the read's task let its client go")
    };
    drop(client);
    let client = full_client(&session);
    let (invoke, rp) = read_request(&first);
    send(&peer, &first.source_mac, ack(invoke, &rp)).await;
    let read = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    let second = receive(&mut requests).await;
    let (invoke, rp) = read_request(&second);
    send(&peer, &second.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert!(
        timeout(Duration::from_millis(100), records.recv())
            .await
            .is_err()
    );
    eventually(WAIT, "invoke IDs are free", async || {
        session.coordinator.active_count().unwrap() == 0
    })
    .await;
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
