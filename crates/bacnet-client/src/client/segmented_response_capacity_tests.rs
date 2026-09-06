use std::collections::HashMap;
use std::sync::Arc;

use bacnet_encoding::apdu::{AbortPdu, Apdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use super::segmented_receive_lifecycle_tests::{
    expect_segment_ack, recv_apdu, response_segment, send_to_client, CLIENT_MAC, SERVER_MAC,
};
use super::{
    BACnetClient, ClientConfig, ResponseLimits, SegKey, SegmentReceiver, SegmentedReceiveState,
};
use crate::tsm::SegmentedResponseAdmission;

const SESSION_CAPACITY: usize = 64;
const UNKNOWN_INVOKE_ID: u8 = u8::MAX;

type RequestTask = JoinHandle<Result<Bytes, Error>>;

struct ActiveSession {
    invoke_id: u8,
    initial_payload: u8,
    task: RequestTask,
}

struct FullCapacityFixture {
    client: Arc<BACnetClient<LoopbackTransport>>,
    server: LoopbackTransport,
    rx: mpsc::Receiver<ReceivedNpdu>,
    sessions: Vec<ActiveSession>,
}

impl FullCapacityFixture {
    async fn start() -> Self {
        let config = ClientConfig {
            apdu_timeout_ms: 30_000,
            apdu_retries: 0,
            ..ClientConfig::default()
        };
        let (client_transport, mut server) =
            LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
        let rx = server.start().await.unwrap();
        let client = Arc::new(BACnetClient::start(config, client_transport).await.unwrap());
        let mut fixture = Self {
            client,
            server,
            rx,
            sessions: Vec::with_capacity(SESSION_CAPACITY),
        };

        for initial_payload in 0..SESSION_CAPACITY as u8 {
            let (invoke_id, task) = fixture.start_pending_request(initial_payload).await;
            send_to_client(
                &fixture.server,
                &response_segment(invoke_id, 0, true, &[initial_payload]),
            )
            .await;
            expect_segment_ack(&mut fixture.rx, invoke_id, 0).await;
            fixture.sessions.push(ActiveSession {
                invoke_id,
                initial_payload,
                task,
            });
        }

        assert_eq!(
            fixture.client.tsm.lock().await.pending_count(),
            SESSION_CAPACITY
        );
        fixture
    }

    async fn start_pending_request(&mut self, request_payload: u8) -> (u8, RequestTask) {
        let client = Arc::clone(&self.client);
        let task = tokio::spawn(async move {
            client
                .confirmed_request(
                    SERVER_MAC,
                    ConfirmedServiceChoice::READ_PROPERTY,
                    &[request_payload],
                )
                .await
        });
        let invoke_id = match recv_apdu(&mut self.rx, "a pending confirmed request").await {
            Apdu::ConfirmedRequest(request) => {
                assert!(!request.segmented);
                assert!(request.segmented_response_accepted);
                request.invoke_id
            }
            other => panic!("expected ConfirmedRequest, got {other:?}"),
        };
        (invoke_id, task)
    }

    async fn shadow_receive_state(&self) -> HashMap<SegKey, SegmentedReceiveState> {
        let owners = {
            let mut tsm = self.client.tsm.lock().await;
            self.sessions
                .iter()
                .map(|session| {
                    let owner = match tsm.admit_segmented_complex_ack(
                        SERVER_MAC,
                        session.invoke_id,
                        1,
                        true,
                    ) {
                        SegmentedResponseAdmission::Active(owner) => owner,
                        other => panic!("active session was not admitted: {other:?}"),
                    };
                    (session.invoke_id, owner)
                })
                .collect::<Vec<_>>()
        };

        owners
            .into_iter()
            .map(|(invoke_id, owner)| {
                let mut receiver = SegmentReceiver::new();
                receiver.receive(0, Bytes::from_static(b"held")).unwrap();
                (
                    (MacAddr::from_slice(SERVER_MAC), invoke_id),
                    SegmentedReceiveState {
                        receiver,
                        owner,
                        reply_mac: MacAddr::from_slice(SERVER_MAC),
                        reply_network: None,
                        expected_next_seq: 1,
                        initial_sequence_number: 0,
                        last_sequence_number: 0,
                        duplicate_count: 0,
                        window_position: 0,
                        actual_window_size: 1,
                        accepted_segments: 1,
                    },
                )
            })
            .collect()
    }

    async fn stop(mut self) {
        for session in &self.sessions {
            send_peer_abort(&self.server, session.invoke_id).await;
        }
        for session in self.sessions {
            expect_abort_result(session.task, AbortReason::BUFFER_OVERFLOW).await;
        }
        assert_eq!(self.client.tsm.lock().await.pending_count(), 0);

        let mut client = Arc::try_unwrap(self.client)
            .unwrap_or_else(|_| panic!("request tasks retained the client"));
        client.stop().await.unwrap();
        self.server.stop().await.unwrap();
    }
}

async fn expect_abort(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    invoke_id: u8,
    reason: AbortReason,
    context: &str,
) {
    match recv_apdu(rx, context).await {
        Apdu::Abort(abort) => {
            assert!(!abort.sent_by_server, "{context}");
            assert_eq!(abort.invoke_id, invoke_id, "{context}");
            assert_eq!(abort.abort_reason, reason, "{context}");
        }
        other => panic!("{context}: expected Abort, got {other:?}"),
    }
}

async fn expect_abort_result(task: RequestTask, expected_reason: AbortReason) {
    match timeout(Duration::from_secs(2), task)
        .await
        .expect("request remained pending")
        .expect("request task panicked")
    {
        Err(Error::Abort { reason }) => assert_eq!(reason, expected_reason.to_raw()),
        other => panic!("expected local Abort indication, got {other:?}"),
    }
}

async fn expect_response(task: RequestTask, expected: &[u8]) {
    let response = timeout(Duration::from_secs(2), task)
        .await
        .expect("request remained pending")
        .expect("request task panicked")
        .expect("request failed");
    assert_eq!(response.as_ref(), expected);
}

async fn send_peer_abort(server: &LoopbackTransport, invoke_id: u8) {
    send_to_client(
        server,
        &Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason: AbortReason::BUFFER_OVERFLOW,
        }),
    )
    .await;
}

#[tokio::test]
async fn full_receive_capacity_aborts_only_the_new_transaction_and_reclaims_its_slot() {
    let mut fixture = FullCapacityFixture::start().await;

    send_to_client(
        &fixture.server,
        &response_segment(UNKNOWN_INVOKE_ID, 0, true, b"unknown"),
    )
    .await;
    expect_abort(
        &mut fixture.rx,
        UNKNOWN_INVOKE_ID,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
        "an unknown transaction while receive capacity is full",
    )
    .await;

    let (invalid_invoke_id, invalid_task) = fixture.start_pending_request(0xA1).await;
    send_to_client(
        &fixture.server,
        &response_segment(invalid_invoke_id, 1, true, b"invalid"),
    )
    .await;
    expect_abort(
        &mut fixture.rx,
        invalid_invoke_id,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
        "an invalid initial sequence while receive capacity is full",
    )
    .await;
    expect_abort_result(invalid_task, AbortReason::INVALID_APDU_IN_THIS_STATE).await;

    let (unsupported_invoke_id, unsupported_task) = fixture.start_pending_request(0xA2).await;
    let mut shadow_state = fixture.shadow_receive_state().await;
    let unsupported_ack = match response_segment(unsupported_invoke_id, 0, true, b"unsupported") {
        Apdu::ComplexAck(ack) => ack,
        _ => unreachable!(),
    };
    BACnetClient::<LoopbackTransport>::handle_segmented_complex_ack(
        &fixture.client.tsm,
        &fixture.client.network,
        &mut shadow_state,
        SERVER_MAC,
        &None,
        unsupported_ack,
        ResponseLimits {
            segmented_response_accepted: false,
            max_reassembly_segments: usize::MAX,
        },
    )
    .await;
    assert_eq!(shadow_state.len(), SESSION_CAPACITY);
    drop(shadow_state);
    expect_abort(
        &mut fixture.rx,
        unsupported_invoke_id,
        AbortReason::SEGMENTATION_NOT_SUPPORTED,
        "an unsupported initial segment while receive capacity is full",
    )
    .await;
    expect_abort_result(unsupported_task, AbortReason::INVALID_APDU_IN_THIS_STATE).await;
    assert_eq!(
        fixture.client.tsm.lock().await.pending_count(),
        SESSION_CAPACITY
    );

    let (rejected_invoke_id, rejected_task) = fixture.start_pending_request(0xA3).await;
    send_to_client(
        &fixture.server,
        &response_segment(rejected_invoke_id, 0, true, b"rejected"),
    )
    .await;
    expect_abort(
        &mut fixture.rx,
        rejected_invoke_id,
        AbortReason::BUFFER_OVERFLOW,
        "a valid initial segment beyond receive capacity",
    )
    .await;
    expect_abort_result(rejected_task, AbortReason::BUFFER_OVERFLOW).await;
    assert_eq!(
        fixture.client.tsm.lock().await.pending_count(),
        SESSION_CAPACITY
    );

    send_to_client(
        &fixture.server,
        &response_segment(rejected_invoke_id, 1, true, b"not retained"),
    )
    .await;
    expect_abort(
        &mut fixture.rx,
        rejected_invoke_id,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
        "a continuation for the rejected newcomer",
    )
    .await;

    let continuing_invoke_id = fixture.sessions[0].invoke_id;
    send_to_client(
        &fixture.server,
        &response_segment(continuing_invoke_id, 1, true, b"continued"),
    )
    .await;
    expect_segment_ack(&mut fixture.rx, continuing_invoke_id, 1).await;

    let reclaimed = fixture.sessions.remove(1);
    send_to_client(
        &fixture.server,
        &response_segment(reclaimed.invoke_id, 1, false, b"complete"),
    )
    .await;
    expect_segment_ack(&mut fixture.rx, reclaimed.invoke_id, 1).await;
    let mut reclaimed_response = vec![reclaimed.initial_payload];
    reclaimed_response.extend_from_slice(b"complete");
    expect_response(reclaimed.task, &reclaimed_response).await;
    assert_eq!(
        fixture.client.tsm.lock().await.pending_count(),
        SESSION_CAPACITY - 1
    );

    let (replacement_invoke_id, replacement_task) = fixture.start_pending_request(0xA4).await;
    send_to_client(
        &fixture.server,
        &response_segment(replacement_invoke_id, 0, true, b"replacement-"),
    )
    .await;
    expect_segment_ack(&mut fixture.rx, replacement_invoke_id, 0).await;
    send_to_client(
        &fixture.server,
        &response_segment(replacement_invoke_id, 1, false, b"response"),
    )
    .await;
    expect_segment_ack(&mut fixture.rx, replacement_invoke_id, 1).await;
    expect_response(replacement_task, b"replacement-response").await;
    assert_eq!(
        fixture.client.tsm.lock().await.pending_count(),
        SESSION_CAPACITY - 1
    );

    fixture.stop().await;
}
