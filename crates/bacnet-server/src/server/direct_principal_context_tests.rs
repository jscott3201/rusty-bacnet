//! Every context's accessors over a real routed direct-TLS envelope (#524):
//! a routed request has no principal, only its forwarding peer's.
use super::*;
use crate::audit_notification::{
    AuditNotificationAuthorizationContext, UnconfirmedAuditNotificationAuthorizationContext,
};
use crate::life_safety::LifeSafetyOperationAuthorizationContext;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::ReinitializedState;

#[tokio::test]
async fn every_context_withholds_the_principal_of_a_routed_request() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut router = f.peer(ca.tls("router")).await;
    // `capture` sends with a routed SNET/SADR source.
    let routed = f.capture(&mut router, &request(true, 1)).await;
    let provenance = routed.provenance;
    let hop = provenance.principal().unwrap();
    let source_mac = MacAddr::from_slice(&PEER_MAC);
    let behind = Some(NpduAddress {
        network: 123,
        mac_address: MacAddr::from_slice(&[3]),
    });
    let sink = ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap();
    let notifications = AuditNotificationRequest {
        notifications: Vec::new(),
    };

    for (source_network, principal) in [(behind, None), (None, Some(hop))] {
        let lso = LifeSafetyOperationAuthorizationContext {
            source_mac: source_mac.clone(),
            source_network: source_network.clone(),
            provenance,
            invoke_id: 1,
            request: LifeSafetyOperationRequest {
                requesting_process_identifier: 1,
                requesting_source: String::new(),
                request: LifeSafetyOperation::RESET,
                object_identifier: None,
            },
        };
        assert_eq!(
            (lso.principal(), lso.forwarding_peer_principal()),
            (principal, Some(hop))
        );
        let audit = AuditNotificationAuthorizationContext {
            source_mac: source_mac.clone(),
            source_network: source_network.clone(),
            provenance,
            invoke_id: 1,
            audit_log_sink: sink,
            request: notifications.clone(),
        };
        assert_eq!(
            (audit.principal(), audit.forwarding_peer_principal()),
            (principal, Some(hop))
        );
        // Debug says whether a principal is present, never its digits.
        let debug = format!("{audit:?}");
        assert!(debug.contains(&format!("principal: {}", principal.is_some())));
        assert!(!debug.contains(&hop.to_string()[..8]), "{debug}");
        assert!(!debug.contains("mac_address"), "{debug}");
        let unconfirmed = UnconfirmedAuditNotificationAuthorizationContext {
            source_mac: source_mac.clone(),
            source_network: source_network.clone(),
            provenance,
            audit_log_sink: sink,
            request: notifications.clone(),
        };
        assert_eq!(
            (
                unconfirmed.principal(),
                unconfirmed.forwarding_peer_principal()
            ),
            (principal, Some(hop))
        );
        let reinitialize = ReinitializeContext::new(
            ReinitializedState::WARMSTART,
            source_mac.clone(),
            source_network,
            provenance,
            1,
        );
        assert_eq!(
            (
                reinitialize.principal(),
                reinitialize.forwarding_peer_principal()
            ),
            (principal, Some(hop))
        );
    }
    f.stop().await;
}
