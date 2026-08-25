use std::collections::HashSet;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

use bacnet_encoding::apdu::{AbortPdu, ComplexAck, ErrorPdu, RejectPdu, SegmentAck};
use bacnet_endpoint_core::coordinator::ReserveError;
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use bytes::Bytes;
use tokio::sync::{mpsc, Notify};

use super::notification_transactions::NotificationReserveError;
use super::*;

const COV_SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
const EVENT_SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;

fn direct_peer(value: u8) -> bacnet_endpoint_core::coordinator::CanonicalPeer {
    canonical_direct_peer(&[value, 0x55])
}

fn simple_ack(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    })
}

fn error(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice,
        error_class: ErrorClass::DEVICE,
        error_code: ErrorCode::OTHER,
        error_data: Bytes::new(),
    })
}

fn reject(invoke_id: u8) -> Apdu {
    Apdu::Reject(RejectPdu {
        invoke_id,
        reject_reason: RejectReason::OTHER,
    })
}

fn abort(invoke_id: u8, sent_by_server: bool) -> Apdu {
    Apdu::Abort(AbortPdu {
        sent_by_server,
        invoke_id,
        abort_reason: AbortReason::OTHER,
    })
}

fn complex_ack(invoke_id: u8, segmented: bool) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented,
        more_follows: segmented,
        invoke_id,
        sequence_number: segmented.then_some(0),
        proposed_window_size: segmented.then_some(1),
        service_choice: COV_SERVICE,
        service_ack: Bytes::new(),
    })
}

fn segment_ack(invoke_id: u8, sent_by_server: bool) -> Apdu {
    Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    })
}

#[test]
fn notification_pool_is_global_bounded_and_generation_safe() {
    let transactions = NotificationTransactions::new();
    let mut operations = Vec::new();
    let mut invoke_ids = HashSet::new();

    for index in 0..=u8::MAX {
        let service = match index % 3 {
            0 => COV_SERVICE,
            1 => ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            _ => EVENT_SERVICE,
        };
        let peer = if index % 2 == 0 {
            direct_peer(index)
        } else {
            canonical_routed_peer(u16::from(index) + 1, &[index, 0xaa])
        };
        let (operation, _receiver) = transactions.reserve(peer, service).unwrap();
        assert!(invoke_ids.insert(operation.invoke_id()));
        operations.push(operation);
    }

    assert_eq!(invoke_ids.len(), 256);
    assert_eq!(transactions.active_count(), 256);
    assert!(matches!(
        transactions.reserve(direct_peer(1), EVENT_SERVICE),
        Err(NotificationReserveError::Coordinator(
            ReserveError::Exhausted
        ))
    ));

    let old_token = operations[0].token();
    let old_invoke_id = operations[0].invoke_id();
    transactions.release_token_for_test(old_token);
    let (replacement, _receiver) = transactions
        .reserve(direct_peer(0xfe), EVENT_SERVICE)
        .unwrap();
    assert_eq!(replacement.invoke_id(), old_invoke_id);
    assert_eq!(transactions.active_count(), 256);

    transactions.release_token_for_test(old_token);
    assert_eq!(
        transactions.active_count(),
        256,
        "stale cleanup must not release the reused generation"
    );
    drop(replacement);
    drop(operations);
    assert_eq!(transactions.active_count(), 0);
}

#[tokio::test]
async fn notification_terminals_complete_exactly_once() {
    let direct_mac = [1, 0x55];

    for (pdu, expected) in [
        (simple_ack(0, COV_SERVICE), CovAckResult::Ack),
        (error(0, EVENT_SERVICE), CovAckResult::Error),
        (reject(0), CovAckResult::Error),
        (abort(0, false), CovAckResult::Error),
    ] {
        let transactions = NotificationTransactions::new();
        let (operation, receiver) = transactions.reserve(direct_peer(1), COV_SERVICE).unwrap();
        let invoke_id = operation.invoke_id();
        let pdu = match pdu {
            Apdu::SimpleAck(mut ack) => {
                ack.invoke_id = invoke_id;
                Apdu::SimpleAck(ack)
            }
            Apdu::Error(mut error) => {
                error.invoke_id = invoke_id;
                Apdu::Error(error)
            }
            Apdu::Reject(mut reject) => {
                reject.invoke_id = invoke_id;
                Apdu::Reject(reject)
            }
            Apdu::Abort(mut abort) => {
                abort.invoke_id = invoke_id;
                Apdu::Abort(abort)
            }
            _ => unreachable!(),
        };

        assert!(transactions.admit_terminal(&direct_mac, None, &pdu));
        assert_eq!(receiver.await.unwrap(), expected);
        assert_eq!(transactions.active_count(), 0);
        assert!(!transactions.admit_terminal(&direct_mac, None, &pdu));
        drop(operation);
    }
}

#[tokio::test]
async fn mismatches_and_nonterminals_leave_notification_pending() {
    let transactions = NotificationTransactions::new();
    let (operation, receiver) = transactions.reserve(direct_peer(1), COV_SERVICE).unwrap();
    let invoke_id = operation.invoke_id();

    for pdu in [
        simple_ack(invoke_id, EVENT_SERVICE),
        abort(invoke_id, true),
        complex_ack(invoke_id, false),
        complex_ack(invoke_id, true),
        segment_ack(invoke_id, false),
        segment_ack(invoke_id, true),
    ] {
        assert!(!transactions.admit_terminal(&[1, 0x55], None, &pdu));
        assert_eq!(transactions.active_count(), 1);
    }
    assert!(!transactions.admit_terminal(&[2, 0x55], None, &simple_ack(invoke_id, COV_SERVICE)));
    assert!(!transactions.admit_terminal(
        &[1, 0x55],
        None,
        &simple_ack(invoke_id.wrapping_add(1), COV_SERVICE)
    ));

    assert!(transactions.admit_terminal(&[1, 0x55], None, &simple_ack(invoke_id, COV_SERVICE)));
    assert_eq!(receiver.await.unwrap(), CovAckResult::Ack);
    assert_eq!(transactions.active_count(), 0);
    drop(operation);
}

#[tokio::test]
async fn routed_identity_ignores_the_immediate_router() {
    let transactions = NotificationTransactions::new();
    let routed_source = NpduAddress {
        network: 400,
        mac_address: MacAddr::from_slice(&[0xaa, 0xbb]),
    };
    let (operation, receiver) = transactions
        .reserve(
            canonical_routed_peer(routed_source.network, &routed_source.mac_address),
            EVENT_SERVICE,
        )
        .unwrap();
    let ack = simple_ack(operation.invoke_id(), EVENT_SERVICE);

    assert!(!transactions.admit_terminal(&[9], None, &ack));
    assert!(!transactions.admit_terminal(
        &[9],
        Some(&NpduAddress {
            network: routed_source.network,
            mac_address: MacAddr::new(),
        }),
        &ack,
    ));
    assert!(transactions.admit_terminal(&[0x44], Some(&routed_source), &ack));
    assert_eq!(receiver.await.unwrap(), CovAckResult::Ack);
    drop(operation);
}

#[tokio::test]
async fn retries_keep_one_lease_and_one_invoke_id() {
    let transactions = NotificationTransactions::new();
    let (operation, receiver) = transactions.reserve(direct_peer(3), COV_SERVICE).unwrap();
    let invoke_id = operation.invoke_id();
    let observed = StdArc::new(StdMutex::new(Vec::new()));
    let observed_by_send = StdArc::clone(&observed);
    let transactions_by_send = Arc::clone(&transactions);

    let result = run_notification_worker(operation, receiver, Duration::ZERO, 3, move |_| {
        observed_by_send.lock().unwrap().push(invoke_id);
        assert_eq!(transactions_by_send.active_count(), 1);
        std::future::ready(Ok::<(), ()>(()))
    })
    .await;

    assert_eq!(result, NotificationWorkerResult::Exhausted);
    assert_eq!(*observed.lock().unwrap(), vec![invoke_id; 4]);
    assert_eq!(transactions.active_count(), 0);
}

#[tokio::test]
async fn final_send_failure_and_worker_abort_clean_up() {
    let transactions = NotificationTransactions::new();
    let (operation, receiver) = transactions.reserve(direct_peer(4), COV_SERVICE).unwrap();
    let result = run_notification_worker(operation, receiver, Duration::ZERO, 1, |_| {
        std::future::ready(Err::<(), ()>(()))
    })
    .await;
    assert_eq!(result, NotificationWorkerResult::Exhausted);
    assert_eq!(transactions.active_count(), 0);

    let (operation, receiver) = transactions.reserve(direct_peer(5), EVENT_SERVICE).unwrap();
    let entered = StdArc::new(Notify::new());
    let blocked = StdArc::new(Notify::new());
    let entered_by_send = StdArc::clone(&entered);
    let blocked_by_send = StdArc::clone(&blocked);
    let worker = tokio::spawn(async move {
        run_notification_worker(operation, receiver, Duration::from_secs(60), 3, move |_| {
            let entered = StdArc::clone(&entered_by_send);
            let blocked = StdArc::clone(&blocked_by_send);
            async move {
                entered.notify_one();
                blocked.notified().await;
                Ok::<(), ()>(())
            }
        })
        .await
    });
    entered.notified().await;
    assert_eq!(transactions.active_count(), 1);
    worker.abort();
    let _ = worker.await;
    assert_eq!(transactions.active_count(), 0);
}

#[tokio::test]
async fn close_drains_waiters_and_rejects_reserve_and_rearm() {
    let transactions = NotificationTransactions::new();
    let (operation, receiver) = transactions.reserve(direct_peer(6), EVENT_SERVICE).unwrap();
    let entered = StdArc::new(Notify::new());
    let entered_by_send = StdArc::clone(&entered);
    let worker = tokio::spawn(async move {
        run_notification_worker(operation, receiver, Duration::from_secs(60), 3, move |_| {
            entered_by_send.notify_one();
            std::future::ready(Ok::<(), ()>(()))
        })
        .await
    });
    entered.notified().await;

    transactions.close();
    assert_eq!(worker.await.unwrap(), NotificationWorkerResult::Closed);
    assert!(transactions.is_closed());
    assert_eq!(transactions.active_count(), 0);
    assert!(matches!(
        transactions.reserve(direct_peer(7), COV_SERVICE),
        Err(NotificationReserveError::Closed)
    ));
}

#[derive(Clone)]
struct IdleTransport {
    local_mac: Vec<u8>,
}

impl Default for IdleTransport {
    fn default() -> Self {
        Self {
            local_mac: vec![127, 0, 0, 1, 0xba, 0xc0],
        }
    }
}

impl TransportPort for IdleTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_sender, receiver) = mpsc::channel(1);
        Ok(receiver)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

#[tokio::test]
async fn dispatch_keeps_segment_and_complex_acks_out_of_notification_completion() {
    let source_mac = MacAddr::from_slice(&[0x31, 0x32]);
    let transactions = NotificationTransactions::new();
    let (operation, receiver) = transactions
        .reserve(canonical_direct_peer(source_mac.as_slice()), COV_SERVICE)
        .unwrap();
    let invoke_id = operation.invoke_id();
    let network = Arc::new(NetworkLayer::new(IdleTransport::default()));
    let db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let config = Arc::new(ServerConfig::default());

    for apdu in [
        segment_ack(invoke_id, false),
        segment_ack(invoke_id, true),
        complex_ack(invoke_id, false),
        complex_ack(invoke_id, true),
    ] {
        BACnetServer::<IdleTransport>::dispatch(
            &db,
            &network,
            &cov_table,
            &seg_ack_senders,
            &seg_send_permits,
            &cov_in_flight,
            &server_tsm,
            &transactions,
            &comm_state,
            &dcc_timer,
            &config,
            source_mac.as_slice(),
            apdu,
            bacnet_network::layer::ReceivedApdu {
                apdu: Bytes::new(),
                source_mac: source_mac.clone(),
                source_network: None,
                is_group: false,
                data_attributes: Vec::new(),
                reply_tx: None,
            },
        )
        .await;
        assert_eq!(transactions.active_count(), 1);
    }

    assert!(transactions.admit_terminal(
        source_mac.as_slice(),
        None,
        &simple_ack(invoke_id, COV_SERVICE)
    ));
    assert_eq!(receiver.await.unwrap(), CovAckResult::Ack);
    drop(operation);
}

#[tokio::test]
async fn server_lifecycle_stop_and_drop_close_notification_transactions() {
    let mut server = BACnetServer::start(
        ServerConfig::default(),
        ObjectDatabase::new(),
        IdleTransport::default(),
    )
    .await
    .unwrap();
    let transactions = Arc::clone(&server.notification_transactions);
    let (_operation, _receiver) = transactions.reserve(direct_peer(8), EVENT_SERVICE).unwrap();
    server.stop().await.unwrap();
    assert!(transactions.is_closed());
    assert_eq!(transactions.active_count(), 0);

    let server = BACnetServer::start(
        ServerConfig::default(),
        ObjectDatabase::new(),
        IdleTransport::default(),
    )
    .await
    .unwrap();
    let transactions = Arc::clone(&server.notification_transactions);
    let (_operation, _receiver) = transactions.reserve(direct_peer(9), COV_SERVICE).unwrap();
    drop(server);
    assert!(transactions.is_closed());
    assert_eq!(transactions.active_count(), 0);
}
