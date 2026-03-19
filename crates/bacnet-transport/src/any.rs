//! Type-erased transport for mixed-transport routing.
//!
//! [`AnyTransport`] wraps all supported BACnet transport types, enabling
//! a single router to manage heterogeneous ports (e.g., BIP + MS/TP).

use bacnet_types::error::Error;
use tokio::sync::mpsc;

use crate::bip::BipTransport;
use crate::bip6::Bip6Transport;
use crate::mstp::{MstpTransport, SerialPort};
use crate::port::{ReceivedNpdu, TransportPort};

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
    /// BACnet/IP over UDP.
    Bip(BipTransport),
    /// MS/TP over RS-485.
    Mstp(MstpTransport<S>),
    /// BACnet/IPv6 over UDP.
    Bip6(Bip6Transport),
    /// BACnet Ethernet over raw LLC frames (Linux only).
    #[cfg(all(feature = "ethernet", target_os = "linux"))]
    Ethernet(EthernetTransport),
    /// BACnet/SC over TLS WebSocket.
    #[cfg(feature = "sc-tls")]
    Sc(Box<ScTransport<TlsWebSocket>>),
}

impl<S: SerialPort + 'static> TransportPort for AnyTransport<S> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        match self {
            Self::Bip(t) => t.start().await,
            Self::Mstp(t) => t.start().await,
            Self::Bip6(t) => t.start().await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.start().await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.start().await,
        }
    }

    async fn stop(&mut self) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.stop().await,
            Self::Mstp(t) => t.stop().await,
            Self::Bip6(t) => t.stop().await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.stop().await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.stop().await,
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.send_unicast(npdu, mac).await,
            Self::Mstp(t) => t.send_unicast(npdu, mac).await,
            Self::Bip6(t) => t.send_unicast(npdu, mac).await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.send_unicast(npdu, mac).await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.send_unicast(npdu, mac).await,
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.send_broadcast(npdu).await,
            Self::Mstp(t) => t.send_broadcast(npdu).await,
            Self::Bip6(t) => t.send_broadcast(npdu).await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.send_broadcast(npdu).await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.send_broadcast(npdu).await,
        }
    }

    fn local_mac(&self) -> &[u8] {
        match self {
            Self::Bip(t) => t.local_mac(),
            Self::Mstp(t) => t.local_mac(),
            Self::Bip6(t) => t.local_mac(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.local_mac(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.local_mac(),
        }
    }

    fn max_apdu_length(&self) -> u16 {
        match self {
            Self::Bip(t) => t.max_apdu_length(),
            Self::Mstp(t) => t.max_apdu_length(),
            Self::Bip6(t) => t.max_apdu_length(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.max_apdu_length(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.max_apdu_length(),
        }
    }
}

impl<S: SerialPort> From<BipTransport> for AnyTransport<S> {
    fn from(t: BipTransport) -> Self {
        Self::Bip(t)
    }
}

impl<S: SerialPort> From<MstpTransport<S>> for AnyTransport<S> {
    fn from(t: MstpTransport<S>) -> Self {
        Self::Mstp(t)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mstp::{LoopbackSerial, MstpConfig};
    use std::net::Ipv4Addr;

    #[test]
    fn any_transport_bip_local_mac() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(bip);
        assert_eq!(any.local_mac().len(), 6);
    }

    #[test]
    fn any_transport_bip_max_apdu() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(bip);
        assert_eq!(any.max_apdu_length(), 1476);
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
    }

    #[test]
    fn any_transport_mstp_max_apdu() {
        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Mstp(mstp);
        assert_eq!(any.max_apdu_length(), 480);
    }

    #[test]
    fn any_transport_from_bip() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = bip.into();
        assert_eq!(any.max_apdu_length(), 1476);
    }

    #[test]
    fn any_transport_from_mstp() {
        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        let any: AnyTransport<LoopbackSerial> = mstp.into();
        assert_eq!(any.max_apdu_length(), 480);
    }

    #[test]
    fn any_transport_bip6_local_mac() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip6(bip6);
        assert_eq!(any.local_mac().len(), 18);
    }

    #[test]
    fn any_transport_bip6_max_apdu() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip6(bip6);
        assert_eq!(any.max_apdu_length(), 1476);
    }

    #[test]
    fn any_transport_from_bip6() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = bip6.into();
        assert_eq!(any.max_apdu_length(), 1476);
    }
}
