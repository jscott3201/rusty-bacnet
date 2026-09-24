use super::batching::{command, delayed};
use super::*;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::port::ReceivedNpdu;

async fn fixture(confirmed: bool) -> (Fixture, mpsc::Sender<ReceivedNpdu>) {
    let (tx, rx) = mpsc::channel(16);
    let transport = CaptureTransport::default();
    *transport.incoming.lock().unwrap() = Some(rx);
    let mut reporter = delayed(60);
    reporter
        .set_issue_confirmed_notifications(confirmed)
        .unwrap();
    (
        try_servers_config(
            vec![reporter],
            &[10],
            Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
            vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
            true,
            1476,
            transport,
        )
        .await
        .unwrap(),
        tx,
    )
}
async fn inject(tx: &mpsc::Sender<ReceivedNpdu>, source: &[u8], apdu: Apdu) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &apdu).unwrap();
    let mut bytes = BytesMut::new();
    encode_npdu(
        &mut bytes,
        &Npdu {
            payload: payload.freeze(),
            ..Default::default()
        },
    )
    .unwrap();
    tx.send(ReceivedNpdu::unverified(
        bytes.freeze(),
        MacAddr::from_slice(source),
        false,
        vec![],
        None,
    ))
    .await
    .unwrap();
}
fn confirmed_frames(f: &CaptureTransport) -> Vec<ConfirmedRequestPdu> {
    f.sent
        .lock()
        .unwrap()
        .iter()
        .map(|bytes| {
            let Apdu::ConfirmedRequest(request) =
                decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap()
            else {
                panic!("confirmed frame")
            };
            request
        })
        .collect()
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_local_send_clears_command_before_ack_then_stop_keeps_only_ack_progress(
) {
    let (mut f, tx) = fixture(true).await;
    write_value(&f.server, None).await;
    command(&f, true).await;
    settle().await;
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .get(&oid(ObjectType::AUDIT_REPORTER, 1))
            .unwrap()
            .read_property(PropertyIdentifier::SEND_NOW, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        f.server.notification_transactions.audit_resources().2,
        62,
        "ordinary batch and immediate command await ACK"
    );
    let captured = f.transport.clone();
    let db = Arc::clone(&f.server.db);
    let owner = Arc::clone(&f.server.notification_transactions);
    let frames = confirmed_frames(&captured);
    assert_eq!(frames.len(), 2);
    let stop = f.server.stop();
    tokio::pin!(stop);
    assert!(futures_util::poll!(stop.as_mut()).is_pending());
    assert!(owner.application_sealed.load(Ordering::Acquire));
    {
        let mut database = db.write().await;
        let reporter = database
            .get_mut(&oid(ObjectType::AUDIT_REPORTER, 1))
            .unwrap();
        for property in [
            PropertyIdentifier::SEND_NOW,
            PropertyIdentifier::MAXIMUM_SEND_DELAY,
        ] {
            assert!(reporter
                .write_property(property, None, PropertyValue::Null, None)
                .is_err());
        }
    }

    for segmented in [false, true] {
        inject(
            &tx,
            SOURCE,
            Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented,
                more_follows: segmented,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: 210 + u8::from(segmented),
                sequence_number: segmented.then_some(0),
                proposed_window_size: segmented.then_some(1),
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                service_request: wp(
                    oid(ObjectType::BINARY_VALUE, 1),
                    PropertyIdentifier::PRESENT_VALUE,
                    vec![0x91, 0],
                    None,
                ),
            }),
        )
        .await;
    }
    settle().await;
    assert!(
        captured.responses.lock().unwrap().is_empty(),
        "no late mutation response or segment ACK"
    );
    assert_eq!(
        db.read()
            .await
            .get(&oid(ObjectType::BINARY_VALUE, 1))
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    for frame in frames {
        inject(
            &tx,
            LOGGER,
            Apdu::SimpleAck(bacnet_encoding::apdu::SimpleAck {
                invoke_id: frame.invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            }),
        )
        .await;
    }
    stop.await.unwrap();
    assert_eq!(owner.audit_resources(), (false, 0, 64));
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_canceled_stop_keeps_first_deadline_and_releases_retained_database() {
    let (mut f, _tx) = fixture(false).await;
    let db = Arc::clone(&f.server.db);
    let queue = Arc::clone(&f.server.target_audit.as_ref().unwrap().batches);
    f.transport.block.store(true, Ordering::Release);
    write_value(&f.server, None).await;
    {
        let stop = f.server.stop();
        tokio::pin!(stop);
        assert!(futures_util::poll!(stop.as_mut()).is_pending());
    }
    let first = queue.stop_deadline().unwrap();
    settle().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    {
        let stop = f.server.stop();
        tokio::pin!(stop);
        assert!(futures_util::poll!(stop.as_mut()).is_pending());
    }
    assert_eq!(queue.stop_deadline(), Some(first));
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    assert!(
        queue.stopped(),
        "scheduler enforces the deadline while caller stop is canceled"
    );
    f.server.stop().await.unwrap();
    assert!(f.server.notification_transactions.workers_empty());
    assert!(db
        .write()
        .await
        .remove(&oid(ObjectType::AUDIT_REPORTER, 1))
        .is_ok());
    assert_eq!(queue.resources().0, 0);
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_drop_cancels_unsent_without_wire_promise_and_releases_membership() {
    let (f, _tx) = fixture(false).await;
    write_value(&f.server, None).await;
    let db = Arc::clone(&f.server.db);
    let queue = Arc::clone(&f.server.target_audit.as_ref().unwrap().batches);
    let transport = f.transport.clone();
    drop(f);
    settle().await;
    assert!(transport.sent.lock().unwrap().is_empty());
    assert_eq!(queue.resources(), (0, 0, 1));
    assert!(db
        .write()
        .await
        .remove(&oid(ObjectType::AUDIT_REPORTER, 1))
        .is_ok());
}
