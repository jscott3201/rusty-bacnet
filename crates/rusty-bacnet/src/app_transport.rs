//! The transport under a Python `BACnetClient` or `BACnetServer`: one of its
//! own links, or a share of a `BACnetRouter`'s application entity (#2071).
//!
//! `AnyTransport` lives below `bacnet-network`, so it can't hold a
//! `RouterApplicationPort`. This enum holds either and passes each
//! `TransportPort` call to the one it holds.

use bacnet_network::router::RouterApplicationPort;
use bacnet_npdu::NpduLimits;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::{AsBip, BipTransport};
use bacnet_transport::port::{
    Bip6Port, BipPort, DataAttribute, GroupDestinations, ReceivedNpdu, TransportPort,
};
use bacnet_types::error::Error;
use tokio::sync::{mpsc, watch};

/// A Python stack's transport: a link it owns, or a router's application
/// entity, whose router owns the links.
pub(crate) enum AppTransport {
    /// A B/IP, B/IPv6, BACnet/SC or MS/TP link of the stack's own.
    Link(AnyTransport),
    /// A share of a running router's application entity.
    Router(RouterApplicationPort),
}

impl AppTransport {
    /// The stack's own link, or `None` on a router.
    pub(crate) fn link(&self) -> Option<&AnyTransport> {
        match self {
            Self::Link(transport) => Some(transport),
            Self::Router(_) => None,
        }
    }
}

impl From<AnyTransport> for AppTransport {
    fn from(transport: AnyTransport) -> Self {
        Self::Link(transport)
    }
}

/// The links the client builders make, for their `build_into`.
macro_rules! from_link {
    ($($link:ty),*) => {$(
        impl From<$link> for AppTransport {
            fn from(transport: $link) -> Self {
                Self::Link(transport.into())
            }
        }
    )*};
}
from_link!(
    BipTransport,
    bacnet_transport::bip6::Bip6Transport,
    bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>
);

/// A router share has no data link of its own, and `DataLink` has no value
/// for one, so it refuses with `Error::Encoding` rather than an
/// `UnsupportedTransport` that would misname it.
impl AsBip for AppTransport {
    fn as_bip(&self) -> Result<&BipTransport, Error> {
        match self {
            Self::Link(transport) => transport.as_bip(),
            Self::Router(_) => Err(Error::Encoding(
                "a stack on a router's application entity has no B/IP link of its own".into(),
            )),
        }
    }
}

/// Pass a call to the transport either variant holds.
macro_rules! each {
    ($self:ident, $t:ident => $call:expr) => {
        match $self {
            AppTransport::Link($t) => $call,
            AppTransport::Router($t) => $call,
        }
    };
}

impl TransportPort for AppTransport {
    fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        each!(self, t => t.bip_broadcast_endpoint())
    }

    fn supports_local_nonrouter_number_controls(&self) -> bool {
        each!(self, t => t.supports_local_nonrouter_number_controls())
    }

    fn bip_port(&self) -> Option<BipPort> {
        each!(self, t => t.bip_port())
    }

    fn bip6_port(&self) -> Option<Bip6Port> {
        each!(self, t => t.bip6_port())
    }

    fn retain_network_port_lease_internal(
        &mut self,
        lease: std::sync::Arc<()>,
    ) -> Result<(), Error> {
        each!(self, t => t.retain_network_port_lease_internal(lease))
    }

    fn prepare_for_routing(&mut self) {
        each!(self, t => t.prepare_for_routing());
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        each!(self, t => t.start().await)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        each!(self, t => t.stop().await)
    }

    fn abort(&mut self) {
        each!(self, t => t.abort());
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        each!(self, t => t.send_unicast(npdu, mac).await)
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        each!(self, t => t.send_unicast_with_data_attributes(npdu, mac, data_attributes).await)
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        each!(self, t => t.send_broadcast(npdu).await)
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        each!(self, t => t.send_broadcast_with_data_attributes(npdu, data_attributes).await)
    }

    fn local_mac(&self) -> &[u8] {
        each!(self, t => t.local_mac())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        each!(self, t => t.local_receive_apdu_capacity())
    }

    fn egress_apdu_limit(&self) -> u16 {
        each!(self, t => t.egress_apdu_limit())
    }

    fn max_npdu(&self) -> NpduLimits {
        each!(self, t => t.max_npdu())
    }

    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        each!(self, t => t.max_npdu_changes())
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        each!(self, t => t.is_broadcast_mac(mac))
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        each!(self, t => t.is_group_destination(mac))
    }

    fn group_destinations(&self) -> GroupDestinations {
        each!(self, t => t.group_destinations())
    }

    fn local_broadcast_destinations(&self) -> GroupDestinations {
        each!(self, t => t.local_broadcast_destinations())
    }
}
