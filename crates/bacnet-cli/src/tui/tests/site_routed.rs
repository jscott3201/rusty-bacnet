//! A routed site (#1790): the client on network 1, an in-process
//! `BACnetRouter`, and the lab device as station 5 on network 2, every link
//! an in-memory loopback. Like the hub site, it runs on paused time.

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::loopback::LoopbackTransport;

use super::*;

/// The lab device's network.
pub(in crate::tui::tests) const ROUTED_NETWORK: u16 = 2;
/// The client's MAC on network 1, shaped like a B/IP address.
const CLIENT_MAC: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
/// The router's MAC on network 1.
const ROUTER_MAC: [u8; 6] = [10, 0, 0, 254, 0xBA, 0xC0];

/// The router and the device behind it.
pub(in crate::tui::tests) struct Routed {
    router: BACnetRouter,
    server: BACnetServer<LoopbackTransport>,
}

impl Routed {
    pub(in crate::tui::tests) async fn stop(mut self) {
        bounded(self.server.stop()).await.unwrap();
        bounded(self.router.stop()).await.unwrap();
    }
}

/// The router, the lab device `instance` behind it as station 5 on network
/// 2, and a client on network 1, with no session.
pub(in crate::tui::tests) async fn routed_parts(
    instance: u32,
) -> (Routed, BACnetClient<LoopbackTransport>) {
    let (client_end, router_near) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), ROUTER_MAC.to_vec());
    let (router_far, station) = LoopbackTransport::pair(vec![0x01], vec![0x05]);
    let ports = vec![
        RouterPort {
            transport: AnyTransport::Loopback(router_near),
            network_number: 1,
        },
        RouterPort {
            transport: AnyTransport::Loopback(router_far),
            network_number: ROUTED_NETWORK,
        },
    ];
    let StartedRouter { router, .. } = bounded(BACnetRouter::start(ports, RouterOptions::new()))
        .await
        .unwrap();
    let server = BACnetServer::generic_builder()
        .device(lab_identity(instance))
        .transport(station)
        .database(lab_database(instance))
        .mutation_policy(bacnet_server::mutation::MutationPolicy::Permissive);
    let server = bounded(Box::pin(server.build())).await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_end)
        .apdu_timeout_ms(2_000);
    let client = bounded(Box::pin(client.build())).await.unwrap();
    (Routed { router, server }, client)
}

impl Routed {
    /// The device behind the router.
    pub(in crate::tui::tests) fn server(&self) -> &BACnetServer<LoopbackTransport> {
        &self.server
    }
}

impl Site {
    /// [`routed_parts`] with a session run with `options` on the client.
    /// The hub holds no port.
    pub(in crate::tui::tests) async fn routed(instance: u32, options: SessionOptions) -> Self {
        let (routed, client) = routed_parts(instance).await;
        let mut site = Self::launch(Arc::new(Hub::default()), Vec::new(), client, options).await;
        site.routed = Some(routed);
        site
    }

    /// The device behind the router.
    pub(in crate::tui::tests) fn routed_server(&self) -> &BACnetServer<LoopbackTransport> {
        &self.routed.as_ref().expect("a routed site").server
    }
}
