//! Conflict policy observes the exact locked registration decision over mTLS.
use super::*;
use crate::sc_hub::ScHubRegistrationKind as Kind;

fn refuse_replacement(kind: Kind) -> ScHubAdmissionDecision {
    match kind {
        Kind::SameUuidSameVmac | Kind::SameUuidMovedVmac => ScHubAdmissionDecision::Deny,
        _ => ScHubAdmissionDecision::Allow,
    }
}

#[tokio::test]
async fn locked_classification_preserves_incumbent_and_standard_collision_capacity() {
    let tls = TestTls::new();
    let clients = clients();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::new(1, 16).unwrap(),
        Some(Arc::new({
            let seen = seen.clone();
            move |input| {
                seen.lock().unwrap().push(input.registration);
                refuse_replacement(input.registration)
            }
        })),
    ));
    let mut incumbent = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    incumbent
        .ws
        .send(request([0x21; 6], [0x11; 16]))
        .await
        .unwrap();
    incumbent.accept().await;
    let (sink, closed, notify) = {
        let map = clients.lock().await;
        let peer = map.get(&[0x21; 6]).unwrap();
        (
            peer.sink.clone(),
            peer.closed.clone(),
            peer.close_notify.clone(),
        )
    };
    for (vmac, uuid, code, denied) in [
        ([0x21; 6], [0x11; 16], ErrorCode::OTHER, 1),
        ([0x22; 6], [0x11; 16], ErrorCode::OTHER, 2),
        ([0x21; 6], [0x33; 16], ErrorCode::NODE_DUPLICATE_VMAC, 2),
        ([0x23; 6], [0x33; 16], ErrorCode::OTHER, 2),
    ] {
        let mut incoming = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
        incoming.ws.send(request(vmac, uuid)).await.unwrap();
        assert_eq!(
            incoming.nak().await,
            ScBvlcResult::Nak {
                result_for: ScFunction::ConnectRequest,
                error_header_marker: 0,
                error_class: if code == ErrorCode::NODE_DUPLICATE_VMAC {
                    ErrorClass::COMMUNICATION
                } else {
                    ErrorClass::RESOURCES
                }
                .to_raw(),
                error_code: code.to_raw(),
                error_details: String::new(),
            }
        );
        incoming.rejected().await;
        assert_eq!(runtime.denied(), denied);
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        let peer = map.get(&[0x21; 6]).unwrap();
        assert!(Arc::ptr_eq(&peer.sink, &sink));
        assert!(Arc::ptr_eq(&peer.closed, &closed));
        assert!(Arc::ptr_eq(&peer.close_notify, &notify));
        assert!(!closed.load(Ordering::Acquire));
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [
            Kind::Initial,
            Kind::SameUuidSameVmac,
            Kind::SameUuidMovedVmac,
            Kind::ConflictingVmac,
            Kind::Initial
        ]
    );
    incumbent
        .ws
        .send(Message::Binary(vec![10, 0, 0x66, 0x77].into()))
        .await
        .unwrap();
    assert_eq!(incumbent.binary().await, [11, 0, 0x66, 0x77]);
    drop(sink);
    incumbent.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn concurrent_refusal_classifies_after_first_commit_same_or_moved_vmac() {
    for moved in [false, true] {
        let tls = TestTls::new();
        let clients = clients();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runtime = Arc::new(AdmissionRuntime::new(
            ScHubAdmissionLimits::new(1, 8).unwrap(),
            Some(Arc::new({
                let seen = seen.clone();
                move |input| {
                    seen.lock().unwrap().push(input.registration);
                    refuse_replacement(input.registration)
                }
            })),
        ));
        let mut first = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
        let mut second = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
        let other = if moved { [0x22; 6] } else { [0x21; 6] };
        let (a, b) = tokio::join!(
            first.ws.send(request([0x21; 6], [0x11; 16])),
            second.ws.send(request(other, [0x11; 16]))
        );
        a.unwrap();
        b.unwrap();
        let (a, b) = tokio::join!(first.binary(), second.binary());
        assert_eq!(
            [a[0], b[0]]
                .iter()
                .filter(|&&function| function == 7)
                .count(),
            1
        );
        let expected = if moved {
            Kind::SameUuidMovedVmac
        } else {
            Kind::SameUuidSameVmac
        };
        assert_eq!(*seen.lock().unwrap(), [Kind::Initial, expected]);
        assert_eq!(runtime.denied(), 1);
        assert_eq!(clients.lock().await.len(), 1);
        let (winner, loser, nak) = if a[0] == 7 {
            (&mut first, &mut second, b)
        } else {
            (&mut second, &mut first, a)
        };
        assert_eq!(nak, [0, 0, 0x22, 0x33, 6, 1, 0, 0, 3, 0, 0]);
        loser.rejected().await;
        winner
            .ws
            .send(Message::Binary(vec![10, 0, 0x66, 0x77].into()))
            .await
            .unwrap();
        assert_eq!(winner.binary().await, [11, 0, 0x66, 0x77]);
        winner.close().await;
        assert!(clients.lock().await.is_empty());
    }
}

#[tokio::test]
async fn replacement_policy_panic_denies_without_retirement_or_capacity_mutation() {
    let tls = TestTls::new();
    let clients = clients();
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::new(1, 8).unwrap(),
        Some(Arc::new(|input| {
            if input.registration != Kind::Initial {
                panic!("policy failed on classified replacement");
            }
            ScHubAdmissionDecision::Allow
        })),
    ));
    let mut incumbent = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    incumbent
        .ws
        .send(request([0x21; 6], [0x11; 16]))
        .await
        .unwrap();
    incumbent.accept().await;
    let sink = clients.lock().await.get(&[0x21; 6]).unwrap().sink.clone();
    let mut other = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    other.ws.send(request([0x22; 6], [0x11; 16])).await.unwrap();
    assert_eq!(
        other.binary().await,
        [0, 0, 0x22, 0x33, 6, 1, 0, 0, 3, 0, 0]
    );
    other.rejected().await;
    assert_eq!(runtime.denied(), 1);
    {
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        let peer = map.get(&[0x21; 6]).unwrap();
        assert!(Arc::ptr_eq(&peer.sink, &sink));
        assert!(!peer.closed.load(Ordering::Acquire));
    }
    drop(sink);
    incumbent.close().await;
}

#[tokio::test]
async fn expired_registry_wait_never_calls_conflict_policy_or_replaces_incumbent() {
    let tls = TestTls::new();
    let clients = clients();
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::default(),
        Some(Arc::new({
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                ScHubAdmissionDecision::Allow
            }
        })),
    ));
    let mut incumbent = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    incumbent
        .ws
        .send(request([0x21; 6], [0x11; 16]))
        .await
        .unwrap();
    incumbent.accept().await;
    let mut other = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    let map = clients.lock().await;
    let sink = map.get(&[0x21; 6]).unwrap().sink.clone();
    tokio::time::pause();
    poll_io(other.ws.send(request([0x22; 6], [0x11; 16])))
        .await
        .unwrap();
    super::super::deadline_test_support::until(|| {
        other.deadline.admission_started.load(Ordering::Acquire)
    })
    .await;
    tokio::time::advance(Duration::from_secs(6)).await;
    other.rejected().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.denied(), 0);
    assert_eq!(map.len(), 1);
    let peer = map.get(&[0x21; 6]).unwrap();
    assert!(Arc::ptr_eq(&peer.sink, &sink));
    assert!(!peer.closed.load(Ordering::Acquire));
    drop(map);
    drop(sink);
    incumbent.close().await;
}

#[test]
fn registration_classification_is_fixed_redacted_labels() {
    for (kind, label) in [
        (Kind::Initial, "Initial"),
        (Kind::SameUuidSameVmac, "SameUuidSameVmac"),
        (Kind::SameUuidMovedVmac, "SameUuidMovedVmac"),
        (Kind::ConflictingVmac, "ConflictingVmac"),
    ] {
        assert_eq!(format!("{kind:?}"), label);
    }
}

#[tokio::test]
async fn known_uuid_moving_onto_another_incumbent_is_a_vmac_conflict() {
    let tls = TestTls::new();
    let clients = clients();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runtime = Arc::new(AdmissionRuntime::new(
        ScHubAdmissionLimits::default(),
        Some(Arc::new({
            let seen = seen.clone();
            move |input| {
                seen.lock().unwrap().push(input.registration);
                refuse_replacement(input.registration)
            }
        })),
    ));
    let mut first = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    first.ws.send(request([0x21; 6], [0x11; 16])).await.unwrap();
    first.accept().await;
    let mut second = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    second
        .ws
        .send(request([0x22; 6], [0x12; 16]))
        .await
        .unwrap();
    second.accept().await;
    let mut incoming = Peer::open(&tls, clients.clone(), runtime.clone(), true).await;
    incoming
        .ws
        .send(request([0x22; 6], [0x11; 16]))
        .await
        .unwrap();
    assert_eq!(
        incoming.nak().await,
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectRequest,
            error_header_marker: 0,
            error_class: ErrorClass::COMMUNICATION.to_raw(),
            error_code: ErrorCode::NODE_DUPLICATE_VMAC.to_raw(),
            error_details: String::new(),
        }
    );
    incoming.rejected().await;
    assert_eq!(
        *seen.lock().unwrap(),
        [Kind::Initial, Kind::Initial, Kind::ConflictingVmac]
    );
    assert_eq!(runtime.denied(), 0);
    assert_eq!(clients.lock().await.len(), 2);
    for peer in [&mut first, &mut second] {
        peer.ws
            .send(Message::Binary(vec![10, 0, 0x66, 0x77].into()))
            .await
            .unwrap();
        assert_eq!(peer.binary().await, [11, 0, 0x66, 0x77]);
        peer.close().await;
    }
    assert!(clients.lock().await.is_empty());
}
