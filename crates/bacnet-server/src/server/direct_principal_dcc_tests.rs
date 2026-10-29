//! DeviceCommunicationControl through `ServerPolicy` over real direct TLS
//! (#1964): the policy tells two CA-valid peers apart by their verified leaf
//! principals, and a routed request names only its forwarding peer.
use super::*;
use crate::device_communication_control::DeviceCommunicationControlAuthorizationContext;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_transport::port::ScPrincipal;
use bacnet_types::enums::EnableDisable;

type Seen = Arc<StdMutex<Vec<(Option<ScPrincipal>, Option<ScPrincipal>)>>>;

fn dcc(mode: EnableDisable, invoke: u8) -> Apdu {
    let mut payload = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: mode,
        password: None,
    }
    .encode(&mut payload)
    .unwrap();
    confirmed(
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        invoke,
        payload.freeze(),
    )
}

/// A policy allowing DCC only from `pinned`, recording what it saw.
fn pinned_policy(pinned: Arc<StdMutex<Option<ScPrincipal>>>, seen: Seen) -> ServerConfig {
    ServerConfig {
        dcc_policy: DccPolicy::LegacyPermissive,
        policy: Some(Arc::new(
            crate::FnPolicy::new().device_communication_control(
                move |context: &DeviceCommunicationControlAuthorizationContext| {
                    let principal = context.principal();
                    seen.lock()
                        .unwrap()
                        .push((principal, context.forwarding_peer_principal()));
                    principal.is_some() && principal == *pinned.lock().unwrap()
                },
            ),
        )),
        ..Default::default()
    }
}

#[tokio::test]
async fn dcc_policy_allows_one_direct_principal_and_denies_another() {
    let ca = TestCa::new();
    let (pinned, seen) = (Arc::default(), Seen::default());
    let config = pinned_policy(Arc::clone(&pinned), seen.clone());
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;

    let mut a = f.peer(ca.tls("a")).await;
    let from_a = f
        .capture_from(&mut a, &dcc(EnableDisable::DISABLE_INITIATION, 1), None)
        .await;
    let a_principal = from_a.provenance.principal().unwrap();
    *pinned.lock().unwrap() = Some(a_principal);
    f.feed(from_a).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);

    // B, from the same CA with the same VMAC and UUID, is denied.
    let mut b = f.peer(ca.tls("b")).await;
    let from_b = f
        .capture_from(&mut b, &dcc(EnableDisable::ENABLE, 2), None)
        .await;
    let b_principal = from_b.provenance.principal().unwrap();
    assert_ne!(a_principal, b_principal);
    f.feed(from_b).await;
    assert!(denied(&f.response().await));
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);

    // Routed through B, the request has no principal; B is its forwarder.
    let routed = f.capture(&mut b, &dcc(EnableDisable::ENABLE, 3)).await;
    f.feed(routed).await;
    assert!(denied(&f.response().await));
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);

    assert_eq!(
        *seen.lock().unwrap(),
        [
            (Some(a_principal), Some(a_principal)),
            (Some(b_principal), Some(b_principal)),
            (None, Some(b_principal)),
        ]
    );
    let counters = f.server.dcc_outcome_counters();
    assert_eq!(
        (counters.accepted_total, counters.policy_denied_total),
        (1, 2)
    );
    f.stop().await;
}
