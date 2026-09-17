//! Admin admission over direct-handler harnesses: real mutual-TLS pairs
//! that bypass only the accept loop (mirrors `peer_uuid_tests`).
//!
//! Rendered AB.6.2.3 cases (accept new UUID, replace known UUID, NAK
//! VMAC collision) keep working under allow-all; the new Admin Deny is a
//! distinct outcome that mutates nothing and wakes nobody.

use super::admission::{AdmissionRuntime, ScHubAdmissionDecision, ScHubAdmissionLimits};
use super::deadline_test_support::{poll_io, request, ClientWs, TestTls};
use super::heartbeat_test_support::clients;
use super::*;
use crate::sc_frame::{decode_sc_bvlc_result, ScBvlcResult};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

struct Peer {
    ws: ClientWs,
    task: JoinHandle<()>,
    deadline: Arc<super::deadlines::ConnectDeadline>,
    active: Arc<AtomicUsize>,
}

impl Peer {
    async fn open(
        tls: &TestTls,
        clients: Clients,
        runtime: Arc<AdmissionRuntime>,
        verified: bool,
    ) -> Self {
        let (server, ws, address, accepted) = tls.pair().await;
        let (write, read) = server.split();
        let deadline = Arc::new(super::deadlines::ConnectDeadline::new(
            accepted + Duration::from_secs(5),
        ));
        let active = Arc::new(AtomicUsize::new(0));
        let permit = super::connection::Admission::new(active.clone(), Duration::from_secs(10));
        let operation = super::deadlines::serve(
            address,
            ([0x10; 6], [0x10; 16]),
            read,
            Arc::new(Mutex::new(write)),
            clients,
            deadline.clone(),
            || {},
            runtime,
            verified,
            super::tasks::Tasks::new().graceful_ctx(),
        );
        let task = tokio::spawn(async move {
            let _permit = permit;
            operation.await;
        });
        Self {
            ws,
            task,
            deadline,
            active,
        }
    }

    async fn binary(&mut self) -> Vec<u8> {
        match poll_io(self.ws.next()).await.unwrap().unwrap() {
            Message::Binary(data) => data.to_vec(),
            other => panic!("expected BVLC binary, got {other:?}"),
        }
    }

    async fn accept(&mut self) {
        let response = decode_sc_message(&self.binary().await).unwrap();
        assert_eq!(response.function, ScFunction::ConnectAccept);
        assert!(self.deadline.is_committed());
        assert!(self.deadline.admission_started.load(Ordering::Acquire));
    }

    async fn nak(&mut self) -> ScBvlcResult {
        let wire = self.binary().await;
        decode_sc_bvlc_result(&decode_sc_message(&wire).unwrap()).unwrap()
    }

    /// The worker finished without committing: NAK-or-close only.
    async fn rejected(&mut self) {
        poll_io(&mut self.task).await.unwrap();
        assert!(matches!(
            poll_io(self.ws.next()).await,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));
        assert!(!self.deadline.is_committed());
        assert_eq!(self.active.load(Ordering::Acquire), 0);
    }

    async fn close(&mut self) {
        self.ws.close(None).await.unwrap();
        poll_io(&mut self.task).await.unwrap();
        assert_eq!(self.active.load(Ordering::Acquire), 0);
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn allow_all() -> Arc<AdmissionRuntime> {
    Arc::new(AdmissionRuntime::new(ScHubAdmissionLimits::default(), None))
}

fn connect_message(vmac: Vmac, uuid: [u8; 16]) -> Message {
    request(vmac, uuid)
}

#[tokio::test]
async fn allow_policy_registers_with_claimed_values_and_reports_input() {
    let tls = TestTls::new();
    let clients = clients();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::default(),
        Some(Arc::new({
            let seen = seen.clone();
            move |input: &ScHubAdmissionInput| {
                seen.lock().unwrap().push(*input);
                ScHubAdmissionDecision::Allow
            }
        })),
    ));
    let mut peer = Peer::open(&tls, clients.clone(), runtime, true).await;
    let mut wire = match connect_message([0x22; 6], [0x22; 16]) {
        Message::Binary(wire) => wire.to_vec(),
        _ => unreachable!(),
    };
    wire[26..28].copy_from_slice(&1200u16.to_be_bytes());
    wire[28..30].copy_from_slice(&480u16.to_be_bytes());
    peer.ws.send(Message::Binary(wire.into())).await.unwrap();
    peer.accept().await;
    {
        let map = clients.lock().await;
        let client = map.get(&[0x22; 6]).unwrap();
        assert_eq!(client.device_uuid, [0x22; 16]);
        assert_eq!((client.max_bvlc, client.max_npdu), (1200, 480));
        assert!(!client.closed.load(Ordering::Acquire));
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].claimed_vmac, [0x22; 6]);
    assert_eq!(seen[0].claimed_uuid, [0x22; 16]);
    assert_eq!(
        (seen[0].claimed_max_bvlc, seen[0].claimed_max_npdu),
        (1200, 480)
    );
    assert!(seen[0].tls_client_verified);
    assert!(seen[0].provenance.is_direct_peer());
    peer.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn deny_leaves_same_uuid_incumbent_untouched_and_live() {
    let tls = TestTls::new();
    let clients = clients();
    const DENIED: Vmac = [0xBB; 6];
    const VMAC: Vmac = [0x22; 6];
    const UUID: [u8; 16] = [0x22; 16];
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::default(),
        Some(Arc::new(|input: &ScHubAdmissionInput| {
            if input.claimed_vmac == DENIED {
                ScHubAdmissionDecision::Deny
            } else {
                ScHubAdmissionDecision::Allow
            }
        })),
    ));
    let mut incumbent = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    incumbent
        .ws
        .send(connect_message(VMAC, UUID))
        .await
        .unwrap();
    incumbent.accept().await;
    let (sink, closed, notify) = {
        let map = clients.lock().await;
        let client = map.get(&VMAC).unwrap();
        (
            client.sink.clone(),
            client.closed.clone(),
            client.close_notify.clone(),
        )
    };

    // Same UUID that would otherwise replace: the deny must win without
    // touching the incumbent.
    let mut denied = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    denied.ws.send(connect_message(DENIED, UUID)).await.unwrap();
    assert_eq!(
        denied.nak().await,
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectRequest,
            error_header_marker: 0,
            error_class: bacnet_types::enums::ErrorClass::RESOURCES.to_raw(),
            error_code: bacnet_types::enums::ErrorCode::OTHER.to_raw(),
            error_details: String::new(),
        }
    );
    denied.rejected().await;
    assert!(
        denied.deadline.admission_started.load(Ordering::Acquire),
        "deny is an admission outcome, not a pre-admission shape rejection"
    );
    assert_eq!(runtime.denied(), 1);

    {
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        let client = map.get(&VMAC).unwrap();
        assert!(Arc::ptr_eq(&client.sink, &sink));
        assert!(Arc::ptr_eq(&client.closed, &closed));
        assert!(Arc::ptr_eq(&client.close_notify, &notify));
        assert_eq!(client.device_uuid, UUID);
        assert!(!closed.load(Ordering::Acquire));
    }

    // The incumbent still relays to a newly admitted peer.
    let mut helper = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    helper
        .ws
        .send(connect_message([0x32; 6], [0x32; 16]))
        .await
        .unwrap();
    helper.accept().await;
    let mut outbound = vec![1, 4, 0, 9];
    outbound.extend_from_slice(&[0x32; 6]);
    outbound.extend_from_slice(&[1, 0]);
    incumbent
        .ws
        .send(Message::Binary(outbound.into()))
        .await
        .unwrap();
    let mut expected = vec![1, 8, 0, 9];
    expected.extend_from_slice(&VMAC);
    expected.extend_from_slice(&[1, 0]);
    assert_eq!(helper.binary().await, expected);
    assert_eq!(runtime.denied(), 1, "relay must not count as a deny");

    drop(sink);
    incumbent.close().await;
    helper.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn same_uuid_replacement_wins_over_capacity_without_deny_count() {
    let tls = TestTls::new();
    let clients = clients();
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits {
            max_clients: 1,
            max_handshakes: 64,
        },
        None,
    ));
    let mut first = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    first
        .ws
        .send(connect_message([0x21; 6], [0x44; 16]))
        .await
        .unwrap();
    first.accept().await;
    let closed = clients.lock().await.get(&[0x21; 6]).unwrap().closed.clone();

    let mut second = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    second
        .ws
        .send(connect_message([0x22; 6], [0x44; 16]))
        .await
        .unwrap();
    second.accept().await;
    assert!(closed.load(Ordering::Acquire));
    {
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&[0x22; 6]).unwrap().device_uuid, [0x44; 16]);
    }
    assert_eq!(runtime.denied(), 0);
    // The superseded worker exits on its own; its cleanup must not evict
    // the replacement.
    poll_io(&mut first.task).await.unwrap();
    assert!(clients.lock().await.contains_key(&[0x22; 6]));
    second.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn different_uuid_vmac_collision_still_naks_duplicate() {
    let tls = TestTls::new();
    let clients = clients();
    let runtime = allow_all();
    let mut owner = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    owner
        .ws
        .send(connect_message([0x21; 6], [0x11; 16]))
        .await
        .unwrap();
    owner.accept().await;
    let sink = clients.lock().await.get(&[0x21; 6]).unwrap().sink.clone();

    let mut colliding = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    colliding
        .ws
        .send(connect_message([0x21; 6], [0x33; 16]))
        .await
        .unwrap();
    assert_eq!(
        colliding.nak().await,
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectRequest,
            error_header_marker: 0,
            error_class: bacnet_types::enums::ErrorClass::COMMUNICATION.to_raw(),
            error_code: bacnet_types::enums::ErrorCode::NODE_DUPLICATE_VMAC.to_raw(),
            error_details: String::new(),
        }
    );
    colliding.rejected().await;
    {
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        let client = map.get(&[0x21; 6]).unwrap();
        assert!(Arc::ptr_eq(&client.sink, &sink));
        assert_eq!(client.device_uuid, [0x11; 16]);
        assert!(!client.closed.load(Ordering::Acquire));
    }
    assert_eq!(runtime.denied(), 0);
    drop(sink);
    owner.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn capacity_and_deny_share_wire_signal_with_separate_accounting() {
    let tls = TestTls::new();
    let clients = clients();
    const DENIED: Vmac = [0xBB; 6];
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits {
            max_clients: 1,
            max_handshakes: 64,
        },
        Some(Arc::new(|input: &ScHubAdmissionInput| {
            if input.claimed_vmac == DENIED {
                ScHubAdmissionDecision::Deny
            } else {
                ScHubAdmissionDecision::Allow
            }
        })),
    ));
    let mut first = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    first
        .ws
        .send(connect_message([0x21; 6], [0x11; 16]))
        .await
        .unwrap();
    first.accept().await;

    // New UUID at capacity: RESOURCES/OTHER, no deny count.
    let mut full = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    full.ws
        .send(connect_message([0x22; 6], [0x22; 16]))
        .await
        .unwrap();
    let capacity_nak = full.binary().await;
    full.rejected().await;
    assert_eq!(runtime.denied(), 0);

    // Denied VMAC: identical wire bytes, counted separately.
    let mut denied = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    denied
        .ws
        .send(connect_message(DENIED, [0x44; 16]))
        .await
        .unwrap();
    let deny_nak = denied.binary().await;
    denied.rejected().await;
    assert_eq!(deny_nak, capacity_nak);
    assert_eq!(
        deny_nak,
        vec![0, 0, 0x22, 0x33, 6, 1, 0, 0, 3, 0, 0],
        "admin deny uses the RESOURCES/OTHER NAK family"
    );
    assert_eq!(runtime.denied(), 1);
    assert_eq!(clients.lock().await.len(), 1);
    first.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn simultaneous_same_uuid_registrations_leave_single_winner() {
    let tls = TestTls::new();
    let clients = clients();
    let runtime = allow_all();
    let mut first = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    let mut second = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    first
        .ws
        .send(connect_message([0x22; 6], [0x22; 16]))
        .await
        .unwrap();
    second
        .ws
        .send(connect_message([0x22; 6], [0x22; 16]))
        .await
        .unwrap();
    first.accept().await;
    second.accept().await;
    // Exactly one worker must lose: replacement closes the loser, whose
    // cleanup must not evict the winner.
    let started = std::time::Instant::now();
    loop {
        let done = first.task.is_finished() as u8 + second.task.is_finished() as u8;
        if done == 1 {
            break;
        }
        assert!(
            done == 0 && started.elapsed() < Duration::from_secs(5),
            "duplicate registrations must leave exactly one winner"
        );
        tokio::task::yield_now().await;
    }
    {
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        let client = map.get(&[0x22; 6]).unwrap();
        assert_eq!(client.device_uuid, [0x22; 16]);
        assert!(!client.closed.load(Ordering::Acquire));
    }
    assert_eq!(runtime.denied(), 0);
    // Close whichever side is still live; the loser is already gone.
    for peer in [&mut first, &mut second] {
        let _ = peer.ws.close(None).await;
    }
    let _ = poll_io(&mut first.task).await;
    let _ = poll_io(&mut second.task).await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn unverified_channel_reaches_policy_as_unverified() {
    let tls = TestTls::new();
    let clients = clients();
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::default(),
        Some(Arc::new(|input: &ScHubAdmissionInput| {
            if input.tls_client_verified {
                ScHubAdmissionDecision::Allow
            } else {
                ScHubAdmissionDecision::Deny
            }
        })),
    ));
    let mut verified = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    verified
        .ws
        .send(connect_message([0x21; 6], [0x11; 16]))
        .await
        .unwrap();
    verified.accept().await;
    let mut unverified = Peer::open(&tls, clients.clone(), runtime.clone(), false).await;
    unverified
        .ws
        .send(connect_message([0x22; 6], [0x22; 16]))
        .await
        .unwrap();
    assert_eq!(
        unverified.nak().await,
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectRequest,
            error_header_marker: 0,
            error_class: bacnet_types::enums::ErrorClass::RESOURCES.to_raw(),
            error_code: bacnet_types::enums::ErrorCode::OTHER.to_raw(),
            error_details: String::new(),
        }
    );
    unverified.rejected().await;
    assert_eq!(runtime.denied(), 1);
    assert_eq!(clients.lock().await.len(), 1);
    verified.close().await;
    assert!(clients.lock().await.is_empty());
}
