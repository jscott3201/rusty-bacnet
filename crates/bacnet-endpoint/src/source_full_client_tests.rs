//! Source Audit of the session's full client (#2617): its records match
//! the narrow requester's for the same operations, on the same session.
use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_services::read_property::ReadPropertyACK;

/// The session's full client, shared with the tasks that call it.
pub(super) type Client = Arc<BACnetClient<TestTransport>>;

pub(super) fn full_client(session: &EndpointSession<TestTransport>) -> Client {
    Arc::new(session.full_client().unwrap())
}

/// A ReadProperty of `target()` on the full client, in a task of its own.
pub(super) fn full_read(
    client: &Client,
    target_mac: &[u8],
    property: PropertyIdentifier,
    index: Option<u32>,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    let client = Arc::clone(client);
    let mac = target_mac.to_vec();
    tokio::spawn(async move {
        client
            .read_property(&mac[..], target(), property, index)
            .await
    })
}

/// `record` with what differs between two operations taken away: the
/// invoke ID and the timestamp.
pub(super) fn fields(mut record: BACnetAuditNotification) -> BACnetAuditNotification {
    record.invoke_id = None;
    record.source_timestamp = None;
    record
}

/// One read answered, its record read and acknowledged when confirmed.
async fn answered_read(
    read: tokio::task::JoinHandle<Result<ReadPropertyACK, Error>>,
    peer: &NetworkLayer<TestTransport>,
    requests: &mut bacnet_network::layer::ApduReceiver,
    sink: &NetworkLayer<TestTransport>,
    records: &mut bacnet_network::layer::ApduReceiver,
    confirmed: bool,
) -> (u8, BACnetAuditNotification) {
    let request = receive(requests).await;
    let (invoke, rp) = read_request(&request);
    send(peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    let envelope = receive(records).await;
    let (record, notification_invoke) = notification(&envelope, confirmed);
    if let Some(invoke_id) = notification_invoke {
        send(
            sink,
            &envelope.source_mac,
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            }),
        )
        .await;
    }
    (invoke, record)
}

#[tokio::test(start_paused = true)]
async fn a_full_client_read_records_what_the_narrow_requester_records() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for confirmed in [false, true] {
            let mut session = session(database(confirmed), role, &sink);
            session.start().await.unwrap();
            let client = full_client(&session);
            let narrow = start_read(
                &session,
                peer.local_mac(),
                PropertyIdentifier::OBJECT_NAME,
                Some(2),
            );
            let (_, narrow) =
                answered_read(narrow, &peer, &mut requests, &sink, &mut records, confirmed).await;
            let full = full_read(
                &client,
                peer.local_mac(),
                PropertyIdentifier::OBJECT_NAME,
                Some(2),
            );
            let (invoke, full) =
                answered_read(full, &peer, &mut requests, &sink, &mut records, confirmed).await;
            assert_eq!(full.invoke_id, Some(invoke));
            let clocked = role == SessionRole::Both;
            assert_stamp(&narrow.source_timestamp, clocked, 0);
            assert_stamp(&full.source_timestamp, clocked, 1);
            assert_eq!(
                fields(full),
                fields(narrow),
                "{role:?} confirmed {confirmed}"
            );
            drop(client);
            session.stop().await.unwrap();
            assert_eq!(session.coordinator.active_count().unwrap(), 0);
        }
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A peer's Error, Reject and Abort, no answer, and an answer that doesn't
/// answer the read are each recorded with the Result the narrow requester
/// gives them; a timeout is TIMEOUT, though the full client reports it as
/// an Abort for its transaction's timeout.
#[tokio::test(start_paused = true)]
async fn a_full_client_reads_failures_carry_the_narrow_requesters_results() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    for case in 0..7 {
        let read = full_read(
            &client,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let envelope = receive(&mut requests).await;
        let (invoke_id, mut rp) = read_request(&envelope);
        let (reply, expected) = match case {
            0 => (
                Some(Apdu::Error(ErrorPdu {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    error_class: ErrorClass::PROPERTY,
                    error_code: ErrorCode::UNKNOWN_PROPERTY,
                    error_data: Bytes::new(),
                })),
                (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            ),
            1 => (
                Some(Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::INVALID_PARAMETER_DATA_TYPE,
                })),
                (
                    ErrorClass::COMMUNICATION,
                    ErrorCode::REJECT_INVALID_PARAMETER_DATA_TYPE,
                ),
            ),
            2 => (
                Some(Apdu::Abort(AbortPdu {
                    invoke_id,
                    sent_by_server: true,
                    abort_reason: AbortReason::OUT_OF_RESOURCES,
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::ABORT_OUT_OF_RESOURCES),
            ),
            3 => (None, (ErrorClass::COMMUNICATION, ErrorCode::TIMEOUT)),
            4 => {
                rp.property_array_index = Some(9);
                (
                    Some(ack(invoke_id, &rp)),
                    (ErrorClass::COMMUNICATION, ErrorCode::OTHER),
                )
            }
            5 => (
                Some(Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    service_ack: Bytes::from_static(&[0]),
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::OTHER),
            ),
            _ => (
                Some(Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::from_raw(200),
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::REJECT_PROPRIETARY),
            ),
        };
        if let Some(reply) = reply {
            send(&peer, &envelope.source_mac, reply).await;
        }
        assert!(read.await.unwrap().is_err(), "case {case}");
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.result, Some(expected), "case {case}");
        assert_eq!(record.invoke_id, Some(invoke_id));
        assert_eq!(
            record.source_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(case))
        );
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A caller that goes away loses nothing: the session's task retries with
/// the same invoke ID and records the answer once.
#[tokio::test(start_paused = true)]
async fn a_full_client_read_whose_caller_goes_away_is_retried_and_recorded_once() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let client = full_client(&session);
    let read = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    let (invoke, rp) = read_request(&first);
    read.abort();
    let _ = read.await;
    let second = receive(&mut requests).await;
    assert_eq!(second.apdu, first.apdu);
    send(&peer, &second.source_mac, ack(invoke, &rp)).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert!(
        timeout(Duration::from_millis(30), records.recv())
            .await
            .is_err()
    );
    eventually(WAIT, "the invoke ID is free", async || {
        session.coordinator.active_count().unwrap() == 0
    })
    .await;
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[path = "source_full_client_services_tests.rs"]
mod services;

#[path = "source_full_client_lifecycle_tests.rs"]
mod lifecycle;

#[path = "source_full_client_slots_tests.rs"]
mod slots;

#[path = "source_full_client_dcc_tests.rs"]
mod dcc;
