//! A router port on a real BACnet/SC hub (#518): broadcast route
//! announcements arrive hub-relayed, with no principal, and a node's
//! control-message rate limit drops a flood end to end.
use super::support::{bounded, run, Certificates};
use crate::server::*;
use bacnet_network::router::control_policy::{
    ControlAuthContext, ControlAuthorizer, ControlPolicy,
};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort};
use bacnet_npdu::NetworkMessageType as Message;
use bacnet_transport::port::{ReceivedNpdu, ScPrincipal};
use bacnet_transport::sc::{ScControlRateLimit, ScTransport};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use std::sync::Mutex as StdMutex;

const ROUTER: [u8; 6] = [2, 0, 0, 0, 0, 0x21];
const ANNOUNCER: [u8; 6] = [2, 0, 0, 0, 0, 0x22];
const STRANGER: [u8; 6] = [2, 0, 0, 0, 0, 0x23];

/// The rule `control_policy`'s module documentation shows, for one SC port:
/// unicast controls need a configured router certificate, broadcast
/// announcements a listed hub-registered VMAC.
fn site(routers: Vec<ScPrincipal>, announcers: Vec<MacAddr>) -> ControlAuthorizer {
    Arc::new(move |context: &ControlAuthContext| {
        let unicast = [
            Message::INITIALIZE_ROUTING_TABLE,
            Message::INITIALIZE_ROUTING_TABLE_ACK,
            Message::REJECT_MESSAGE_TO_NETWORK,
            Message::I_COULD_BE_ROUTER_TO_NETWORK,
        ]
        .contains(&context.message_type);
        if unicast {
            context
                .sending_peer_principal()
                .is_some_and(|peer| routers.contains(&peer))
        } else {
            announcers.contains(&context.source_mac)
        }
    })
}

async fn node(url: &str, tls: ScNodeTlsConfig, vmac: [u8; 6]) -> ScTransport<TlsWebSocket> {
    let ws = bounded(TlsWebSocket::connect(url, tls)).await.unwrap();
    ScTransport::new(ws, vmac).with_device_uuid([vmac[5]; 16])
}

async fn announcer(
    url: &str,
    tls: ScNodeTlsConfig,
    vmac: [u8; 6],
) -> (ScTransport<TlsWebSocket>, mpsc::Receiver<ReceivedNpdu>) {
    let mut transport = node(url, tls, vmac).await;
    let received = bounded(transport.start()).await.unwrap();
    (transport, received)
}

fn i_am_router(network: u16) -> Vec<u8> {
    let [high, low] = network.to_be_bytes();
    vec![0x01, 0x80, 0x01, high, low]
}

/// A broadcast Who-Is: an APDU the router hands to its local receiver,
/// behind everything the same sender broadcast before it.
const WHO_IS: [u8; 4] = [0x01, 0x00, 0x10, 0x08];

#[tokio::test]
async fn sc_hub_relayed_route_announcements_carry_no_principal() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let record = seen.clone();
        let rule = site(Vec::new(), vec![MacAddr::from_slice(&ANNOUNCER)]);
        let authorizer: ControlAuthorizer = Arc::new(move |context| {
            record.lock().unwrap().push((
                context.sending_peer_principal(),
                context.provenance.is_relayed_origin(),
            ));
            rule(context)
        });
        let started = BACnetRouter::start(
            vec![RouterPort {
                transport: node(&f.url, certs.clients[0].clone(), ROUTER).await,
                network_number: 10,
            }],
            RouterOptions::new()
                .control_policy(ControlPolicy::Hardened)
                .control_authorizer(authorizer),
        )
        .await
        .unwrap();
        let (mut router, mut apdus) = (started.router, started.apdus);
        let (mut listed, _a) = announcer(&f.url, certs.clients[1].clone(), ANNOUNCER).await;
        let (mut stranger, _b) = announcer(&f.url, certs.clients[2].clone(), STRANGER).await;

        for (from, network) in [(&listed, 300), (&stranger, 400)] {
            bounded(from.send_broadcast(&i_am_router(network)))
                .await
                .unwrap();
            bounded(from.send_broadcast(&WHO_IS)).await.unwrap();
            bounded(apdus.recv()).await.unwrap();
        }
        {
            let table = router.table();
            assert!(table.lookup(300).is_some(), "a listed announcer is learned");
            assert!(table.lookup(400).is_none(), "an unlisted one is not");
        }
        // Both hub-relayed, neither with a certificate to check.
        assert_eq!(*seen.lock().unwrap(), [(None, true), (None, true)]);

        for transport in [&mut listed, &mut stranger] {
            bounded(transport.stop()).await.unwrap();
        }
        router.stop().await;
    })
    .await;
}

#[tokio::test]
async fn a_control_flood_through_the_hub_stops_at_the_node_rate_limit() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        let started = BACnetRouter::start(
            vec![RouterPort {
                transport: node(&f.url, certs.clients[0].clone(), ROUTER)
                    .await
                    .with_control_rate_limit(ScControlRateLimit::new(4, 1)),
                network_number: 10,
            }],
            RouterOptions::new(),
        )
        .await
        .unwrap();
        let (mut router, mut apdus) = (started.router, started.apdus);
        let (mut flooder, _rx) = announcer(&f.url, certs.clients[1].clone(), ANNOUNCER).await;
        for network in 1000..1010 {
            bounded(flooder.send_broadcast(&i_am_router(network)))
                .await
                .unwrap();
        }
        // The Who-Is, an APDU, is never charged and arrives behind the flood.
        bounded(flooder.send_broadcast(&WHO_IS)).await.unwrap();
        bounded(apdus.recv()).await.unwrap();
        let learned = {
            let table = router.table();
            (1000..1010)
                .filter(|network| table.lookup(*network).is_some())
                .count()
        };
        // The burst of 4, and at most one refill if the run took a second.
        assert!((4..=5).contains(&learned), "learned {learned}");
        assert_eq!(router.control_snapshot().i_am.allow_total as usize, learned);
        bounded(flooder.stop()).await.unwrap();
        router.stop().await;
    })
    .await;
}
