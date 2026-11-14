//! The object-safe view of a transport a [`TestTransport`](super::TestTransport)
//! stands in front of, so one concrete type serves any inner transport.

use std::future::Future;
use std::net::SocketAddrV4;
use std::pin::Pin;
use std::sync::Arc;

use bacnet_npdu::NpduLimits;
use bacnet_types::error::Error;
use tokio::sync::{mpsc, watch};

use crate::port::{
    Bip6Port, BipPort, DataAttribute, GroupDestinations, ReceivedNpdu, TransportPort,
};

pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// [`TransportPort`] with boxed futures. Every `T: TransportPort + 'static`
/// has it, so an inner transport is a real link, a test double, or another
/// test transport.
pub(super) trait InnerPort: Send + Sync {
    fn start(&mut self) -> BoxFuture<'_, Result<mpsc::Receiver<ReceivedNpdu>, Error>>;
    fn stop(&mut self) -> BoxFuture<'_, Result<(), Error>>;
    fn abort(&mut self);
    fn prepare_for_routing(&mut self);
    fn send_unicast<'a>(
        &'a self,
        npdu: &'a [u8],
        mac: &'a [u8],
        attributes: &'a [DataAttribute],
    ) -> BoxFuture<'a, Result<(), Error>>;
    fn send_broadcast<'a>(
        &'a self,
        npdu: &'a [u8],
        attributes: &'a [DataAttribute],
    ) -> BoxFuture<'a, Result<(), Error>>;
    fn local_mac(&self) -> &[u8];
    fn receive_capacity(&self) -> u16;
    fn egress_limit(&self) -> u16;
    fn max_npdu(&self) -> NpduLimits;
    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>>;
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool;
    fn is_group_destination(&self, mac: &[u8]) -> bool;
    fn group_destinations(&self) -> GroupDestinations;
    fn local_broadcast_destinations(&self) -> GroupDestinations;
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4>;
    fn bip_port(&self) -> Option<BipPort>;
    fn bip6_port(&self) -> Option<Bip6Port>;
    fn number_controls(&self) -> bool;
    fn retain_lease(&mut self, lease: Arc<()>) -> Result<(), Error>;
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: TransportPort + 'static> InnerPort for T {
    fn start(&mut self) -> BoxFuture<'_, Result<mpsc::Receiver<ReceivedNpdu>, Error>> {
        Box::pin(TransportPort::start(self))
    }
    fn stop(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(TransportPort::stop(self))
    }
    fn abort(&mut self) {
        TransportPort::abort(self);
    }
    fn prepare_for_routing(&mut self) {
        TransportPort::prepare_for_routing(self);
    }
    fn send_unicast<'a>(
        &'a self,
        npdu: &'a [u8],
        mac: &'a [u8],
        attributes: &'a [DataAttribute],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(TransportPort::send_unicast_with_data_attributes(
            self, npdu, mac, attributes,
        ))
    }
    fn send_broadcast<'a>(
        &'a self,
        npdu: &'a [u8],
        attributes: &'a [DataAttribute],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(TransportPort::send_broadcast_with_data_attributes(
            self, npdu, attributes,
        ))
    }
    fn local_mac(&self) -> &[u8] {
        TransportPort::local_mac(self)
    }
    fn receive_capacity(&self) -> u16 {
        TransportPort::local_receive_apdu_capacity(self)
    }
    fn egress_limit(&self) -> u16 {
        TransportPort::egress_apdu_limit(self)
    }
    fn max_npdu(&self) -> NpduLimits {
        TransportPort::max_npdu(self)
    }
    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        TransportPort::max_npdu_changes(self)
    }
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        TransportPort::is_broadcast_mac(self, mac)
    }
    fn is_group_destination(&self, mac: &[u8]) -> bool {
        TransportPort::is_group_destination(self, mac)
    }
    fn group_destinations(&self) -> GroupDestinations {
        TransportPort::group_destinations(self)
    }
    fn local_broadcast_destinations(&self) -> GroupDestinations {
        TransportPort::local_broadcast_destinations(self)
    }
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        TransportPort::bip_broadcast_endpoint(self)
    }
    fn bip_port(&self) -> Option<BipPort> {
        TransportPort::bip_port(self)
    }
    fn bip6_port(&self) -> Option<Bip6Port> {
        TransportPort::bip6_port(self)
    }
    fn number_controls(&self) -> bool {
        TransportPort::supports_local_nonrouter_number_controls(self)
    }
    fn retain_lease(&mut self, lease: Arc<()>) -> Result<(), Error> {
        TransportPort::retain_network_port_lease_internal(self, lease)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
