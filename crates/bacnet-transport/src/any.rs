//! Type-erased transport for mixed-transport routing.
//!
//! [`AnyTransport`] wraps all supported BACnet transport types, enabling
//! a single router to manage heterogeneous ports (e.g., BIP + MS/TP).

use bacnet_types::data_link::DataLink;
use bacnet_types::error::Error;
use tokio::sync::mpsc;

use crate::bip::{AsBip, BipTransport};
#[cfg(feature = "ipv6")]
use crate::bip6::Bip6Transport;
use crate::loopback::LoopbackTransport;
use crate::mstp::{MstpTransport, SerialPort};
use crate::port::{DataAttribute, ReceivedNpdu, TransportPort};

#[cfg(all(feature = "ethernet", target_os = "linux"))]
use crate::ethernet::EthernetTransport;

#[cfg(feature = "sc-tls")]
use crate::sc::ScTransport;
#[cfg(feature = "sc-tls")]
use crate::sc_tls::TlsWebSocket;

/// A transport that can be any supported BACnet transport type.
///
/// Enables mixed-transport routing (e.g., BIP + MS/TP on the same router).
pub enum AnyTransport<S: SerialPort + 'static> {
    /// BACnet/IP over UDP. Boxed, like `Sc`, because it is several times larger
    /// than the other variants.
    Bip(Box<BipTransport>),
    /// MS/TP over RS-485.
    Mstp(MstpTransport<S>),
    /// BACnet/IPv6 over UDP.
    #[cfg(feature = "ipv6")]
    Bip6(Bip6Transport),
    /// BACnet Ethernet over raw LLC frames (Linux only).
    #[cfg(all(feature = "ethernet", target_os = "linux"))]
    Ethernet(EthernetTransport),
    /// BACnet/SC over TLS WebSocket.
    #[cfg(feature = "sc-tls")]
    Sc(Box<ScTransport<TlsWebSocket>>),
    /// In-process loopback (for gateway client/server composition).
    Loopback(LoopbackTransport),
}

impl<S: SerialPort + 'static> TransportPort for AnyTransport<S> {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        match self {
            Self::Bip(t) => t.supports_local_nonrouter_number_controls(),
            Self::Mstp(t) => t.supports_local_nonrouter_number_controls(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.supports_local_nonrouter_number_controls(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.supports_local_nonrouter_number_controls(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.supports_local_nonrouter_number_controls(),
            Self::Loopback(t) => t.supports_local_nonrouter_number_controls(),
        }
    }

    fn retain_network_port_lease_internal(
        &mut self,
        lease: std::sync::Arc<()>,
    ) -> Result<(), Error> {
        match self {
            Self::Bip(transport) => transport.retain_network_port_lease_internal(lease),
            _ => Err(Error::Encoding("registered ports require B/IP".into())),
        }
    }
    fn bip_port(&self) -> Option<crate::port::BipPort> {
        match self {
            Self::Bip(transport) => transport.bip_port(),
            _ => None,
        }
    }
    fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        match self {
            Self::Bip(transport) => transport.bip_broadcast_endpoint(),
            _ => None,
        }
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        match self {
            Self::Bip(t) => t.start().await,
            Self::Mstp(t) => t.start().await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.start().await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.start().await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.start().await,
            Self::Loopback(t) => t.start().await,
        }
    }

    async fn stop(&mut self) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.stop().await,
            Self::Mstp(t) => t.stop().await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.stop().await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.stop().await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.stop().await,
            Self::Loopback(t) => t.stop().await,
        }
    }

    fn abort(&mut self) {
        match self {
            Self::Bip(t) => t.abort(),
            Self::Mstp(t) => t.abort(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.abort(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.abort(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.abort(),
            Self::Loopback(t) => t.abort(),
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.send_unicast(npdu, mac).await,
            Self::Mstp(t) => t.send_unicast(npdu, mac).await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.send_unicast(npdu, mac).await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.send_unicast(npdu, mac).await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.send_unicast(npdu, mac).await,
            Self::Loopback(t) => t.send_unicast(npdu, mac).await,
        }
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        match self {
            Self::Bip(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            Self::Mstp(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            Self::Loopback(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.send_broadcast(npdu).await,
            Self::Mstp(t) => t.send_broadcast(npdu).await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.send_broadcast(npdu).await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.send_broadcast(npdu).await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.send_broadcast(npdu).await,
            Self::Loopback(t) => t.send_broadcast(npdu).await,
        }
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        match self {
            Self::Bip(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            Self::Mstp(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            Self::Loopback(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
        }
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        match self {
            Self::Bip(t) => t.local_receive_apdu_capacity(),
            Self::Mstp(t) => t.local_receive_apdu_capacity(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.local_receive_apdu_capacity(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.local_receive_apdu_capacity(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.local_receive_apdu_capacity(),
            Self::Loopback(t) => t.local_receive_apdu_capacity(),
        }
    }

    fn local_mac(&self) -> &[u8] {
        match self {
            Self::Bip(t) => t.local_mac(),
            Self::Mstp(t) => t.local_mac(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.local_mac(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.local_mac(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.local_mac(),
            Self::Loopback(t) => t.local_mac(),
        }
    }

    fn egress_apdu_limit(&self) -> u16 {
        match self {
            Self::Bip(t) => t.egress_apdu_limit(),
            Self::Mstp(t) => t.egress_apdu_limit(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.egress_apdu_limit(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.egress_apdu_limit(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.egress_apdu_limit(),
            Self::Loopback(t) => t.egress_apdu_limit(),
        }
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        match self {
            Self::Bip(t) => t.is_broadcast_mac(mac),
            Self::Mstp(t) => t.is_broadcast_mac(mac),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.is_broadcast_mac(mac),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.is_broadcast_mac(mac),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.is_broadcast_mac(mac),
            Self::Loopback(t) => t.is_broadcast_mac(mac),
        }
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        match self {
            Self::Bip(t) => t.is_group_destination(mac),
            Self::Mstp(t) => t.is_group_destination(mac),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.is_group_destination(mac),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.is_group_destination(mac),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.is_group_destination(mac),
            Self::Loopback(t) => t.is_group_destination(mac),
        }
    }

    fn group_destinations(&self) -> crate::port::GroupDestinations {
        match self {
            Self::Bip(t) => t.group_destinations(),
            Self::Mstp(t) => t.group_destinations(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.group_destinations(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.group_destinations(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.group_destinations(),
            Self::Loopback(t) => t.group_destinations(),
        }
    }
}

impl<S: SerialPort + 'static> AnyTransport<S> {
    /// The data link this variant carries, for errors.
    fn data_link(&self) -> DataLink {
        match self {
            Self::Bip(_) => DataLink::Bip,
            Self::Mstp(_) => DataLink::Mstp,
            #[cfg(feature = "ipv6")]
            Self::Bip6(_) => DataLink::Bip6,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(_) => DataLink::Ethernet,
            #[cfg(feature = "sc-tls")]
            Self::Sc(_) => DataLink::Sc,
            Self::Loopback(_) => DataLink::Loopback,
        }
    }
}

impl<S: SerialPort + 'static> AsBip for AnyTransport<S> {
    /// The [`Bip`](Self::Bip) variant's transport.
    ///
    /// # Errors
    ///
    /// [`Error::UnsupportedTransport`] naming the variant's data link for
    /// every other variant.
    fn as_bip(&self) -> Result<&BipTransport, Error> {
        match self {
            Self::Bip(transport) => Ok(transport),
            other => Err(Error::UnsupportedTransport {
                required: DataLink::Bip,
                actual: other.data_link(),
            }),
        }
    }
}

impl<S: SerialPort> From<BipTransport> for AnyTransport<S> {
    fn from(t: BipTransport) -> Self {
        Self::Bip(Box::new(t))
    }
}

impl<S: SerialPort> From<MstpTransport<S>> for AnyTransport<S> {
    fn from(t: MstpTransport<S>) -> Self {
        Self::Mstp(t)
    }
}

#[cfg(feature = "ipv6")]
impl<S: SerialPort> From<Bip6Transport> for AnyTransport<S> {
    fn from(t: Bip6Transport) -> Self {
        Self::Bip6(t)
    }
}

#[cfg(all(feature = "ethernet", target_os = "linux"))]
impl<S: SerialPort> From<EthernetTransport> for AnyTransport<S> {
    fn from(t: EthernetTransport) -> Self {
        Self::Ethernet(t)
    }
}

#[cfg(feature = "sc-tls")]
impl<S: SerialPort> From<ScTransport<TlsWebSocket>> for AnyTransport<S> {
    fn from(t: ScTransport<TlsWebSocket>) -> Self {
        Self::Sc(Box::new(t))
    }
}

impl<S: SerialPort> From<LoopbackTransport> for AnyTransport<S> {
    fn from(t: LoopbackTransport) -> Self {
        Self::Loopback(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mstp::{LoopbackSerial, MstpConfig};
    use std::net::Ipv4Addr;

    #[test]
    fn any_transport_bip_local_mac() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(Box::new(bip));
        assert_eq!(any.local_mac().len(), 6);
        assert!(any.supports_local_nonrouter_number_controls());
    }

    #[test]
    fn any_transport_bip_max_apdu() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(Box::new(bip));
        assert_eq!(any.egress_apdu_limit(), 1476);
        assert_eq!(any.local_receive_apdu_capacity(), 1476);
    }

    #[test]
    fn any_transport_mstp_local_mac() {
        let (serial, _) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 42,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mstp = MstpTransport::new(serial, config);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Mstp(mstp);
        assert_eq!(any.local_mac(), &[42]);
        assert!(any.supports_local_nonrouter_number_controls());
        assert!(any.bip_port().is_none());
    }

    #[test]
    fn any_transport_mstp_max_apdu() {
        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Mstp(mstp);
        assert_eq!(any.egress_apdu_limit(), 480);
        assert_eq!(any.local_receive_apdu_capacity(), 480);
    }

    #[test]
    fn any_transport_from_bip() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = bip.into();
        assert_eq!(any.egress_apdu_limit(), 1476);
        assert_eq!(any.local_receive_apdu_capacity(), 1476);
    }

    /// `as_bip` refuses `any`, naming `link` as the data link it carries.
    fn assert_refused(any: &AnyTransport<LoopbackSerial>, link: DataLink) {
        assert_eq!(any.data_link(), link);
        assert!(matches!(
            any.as_bip(),
            Err(Error::UnsupportedTransport { required: DataLink::Bip, actual }) if actual == link
        ));
    }

    #[test]
    fn as_bip_lends_the_bip_variant_and_names_every_other_data_link() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = bip.into();
        assert_eq!(any.data_link(), DataLink::Bip);
        let lent = any.as_bip().expect("the Bip variant lends its transport");
        assert_eq!(lent.local_mac(), any.local_mac());

        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        assert_refused(&mstp.into(), DataLink::Mstp);
        let (loopback, _) = LoopbackTransport::pair(vec![1], vec![2]);
        assert_refused(&loopback.into(), DataLink::Loopback);
        #[cfg(feature = "ipv6")]
        {
            let bip6 = Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
            assert_refused(&bip6.into(), DataLink::Bip6);
        }
        #[cfg(all(feature = "ethernet", target_os = "linux"))]
        assert_refused(&EthernetTransport::new("lo").into(), DataLink::Ethernet);
    }

    #[cfg(feature = "sc-tls")]
    #[tokio::test]
    async fn as_bip_names_the_sc_data_link() {
        let (_hub_end, ws) = crate::sc_hub::ws_limits_test_support::initiating_pair().await;
        assert_refused(&ScTransport::new(ws, [0x02; 6]).into(), DataLink::Sc);
    }

    #[test]
    fn any_transport_from_mstp() {
        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        let any: AnyTransport<LoopbackSerial> = mstp.into();
        assert_eq!(any.egress_apdu_limit(), 480);
        assert_eq!(any.local_receive_apdu_capacity(), 480);
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn any_transport_bip6_local_mac() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip6(bip6);
        assert_eq!(any.local_mac().len(), 18);
        assert!(any.supports_local_nonrouter_number_controls());
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn any_transport_bip6_max_apdu() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip6(bip6);
        assert_eq!(any.egress_apdu_limit(), 1476);
        assert_eq!(any.local_receive_apdu_capacity(), 1476);
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn any_transport_from_bip6() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = bip6.into();
        assert_eq!(any.egress_apdu_limit(), 1476);
        assert_eq!(any.local_receive_apdu_capacity(), 1476);
    }
}
