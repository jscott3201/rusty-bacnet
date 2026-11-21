//! A routed site (#1790): the client on network 1, an in-process
//! `BACnetRouter`, and the lab device as station 5 on network 2, every link
//! an in-memory loopback. Like the hub site, it runs on paused time.

use std::pin::Pin;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::loopback::LoopbackTransport;

use super::*;
use crate::heap::boxed;

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
///
/// Boxed, as each start in it is: a debug build keeps a copy of every
/// future a test awaits in the test's poll frame, and the router's start
/// runs under all of them (#2656).
pub(in crate::tui::tests) fn routed_parts(
    instance: u32,
) -> Pin<Box<impl Future<Output = (Routed, BACnetClient<LoopbackTransport>)>>> {
    boxed(move || start_routed_parts(instance))
}

async fn start_routed_parts(instance: u32) -> (Routed, BACnetClient<LoopbackTransport>) {
    let (client_end, router_near) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), ROUTER_MAC.to_vec());
    let (router_far, station) = LoopbackTransport::pair(vec![0x01], vec![0x05]);
    let ports = vec![
        RouterPort::new(AnyTransport::Loopback(router_near), 1),
        RouterPort::new(AnyTransport::Loopback(router_far), ROUTED_NETWORK),
    ];
    let StartedRouter { router, .. } =
        bounded(boxed(|| BACnetRouter::start(ports, RouterOptions::new())))
            .await
            .unwrap();
    let server = BACnetServer::generic_builder()
        .device(lab_identity(instance))
        .transport(station)
        .database(lab_database(instance))
        .mutation_policy(bacnet_server::mutation::MutationPolicy::Permissive);
    let server = bounded(boxed(|| server.build())).await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_end)
        .apdu_timeout_ms(2_000);
    let client = bounded(boxed(|| client.build())).await.unwrap();
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
    /// The hub holds no port. Boxed, as [`routed_parts`] is.
    pub(in crate::tui::tests) fn routed(
        instance: u32,
        options: SessionOptions,
    ) -> Pin<Box<impl Future<Output = Self>>> {
        boxed(move || async move {
            let (routed, client) = routed_parts(instance).await;
            let hub = Arc::new(Hub::default());
            let mut site = boxed(|| Self::launch(hub, Vec::new(), client, options)).await;
            site.routed = Some(routed);
            site
        })
    }

    /// The device behind the router.
    pub(in crate::tui::tests) fn routed_server(&self) -> &BACnetServer<LoopbackTransport> {
        &self.routed.as_ref().expect("a routed site").server
    }
}
