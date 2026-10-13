//! One socket/port, selected normal-mode source and receive authority.

use std::{
    io,
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6},
};
use tokio::net::UdpSocket;

use super::{
    link::SelectedLink, send::SelectedSender, BACNET_IPV6_MULTICAST_LINK_LOCAL,
    BACNET_IPV6_MULTICAST_ORG_LOCAL, BACNET_IPV6_MULTICAST_SITE_LOCAL,
};
use crate::udp_metadata::{DestinationReceiver, IpVersion, ReceivedDatagram};

pub(super) struct Bip6Socket {
    udp: UdpSocket,
    receiver: DestinationReceiver,
    pub selection: Option<SelectedLink>,
    sender: Option<SelectedSender>,
}

/// The unbound B/IPv6 socket. Only an explicit port sets SO_REUSEADDR: Linux
/// would otherwise hand an ephemeral bind a port another SO_REUSEADDR socket
/// owns, and unicast to it would reach only one of them (#892). An ephemeral
/// port claims exclusive use where the OS has a way to (`port_ownership`, #950).
fn udp_socket(port: u16) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_only_v6(true)?;
    if port != 0 {
        socket.set_reuse_address(true)?;
    } else {
        crate::port_ownership::claim_exclusive(&socket)?;
    }
    socket.set_nonblocking(true)?;
    socket.set_multicast_loop_v6(true)?;
    Ok(socket)
}

impl Bip6Socket {
    pub async fn bind(
        requested: Ipv6Addr,
        port: u16,
        foreign: Option<SocketAddrV6>,
    ) -> io::Result<Self> {
        let selection = if foreign.is_none() {
            Some(SelectedLink::resolve(requested).await?)
        } else {
            None
        };
        let socket = udp_socket(port)?;
        let bound = if let Some(bbmd) = foreign {
            foreign_bind_address(requested, port, bbmd)?
        } else {
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0)
        };
        socket.bind(&bound.into())?;
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V6)?;
        let sender = if let Some(link) = selection {
            socket.set_multicast_if_v6(link.index)?;
            for group in [
                BACNET_IPV6_MULTICAST_LINK_LOCAL,
                BACNET_IPV6_MULTICAST_SITE_LOCAL,
                BACNET_IPV6_MULTICAST_ORG_LOCAL,
            ] {
                socket.join_multicast_v6(&group, link.index)?;
            }
            Some(SelectedSender::new(&socket)?)
        } else {
            None
        };
        Ok(Self {
            udp: UdpSocket::from_std(socket.into())?,
            receiver,
            selection,
            sender,
        })
    }

    pub fn local_address(&self) -> io::Result<SocketAddrV6> {
        match self.udp.local_addr()? {
            SocketAddr::V6(address) => Ok(address),
            _ => Err(io::Error::other("IPv6 socket returned a non-IPv6 address")),
        }
    }
    pub fn local_port(&self) -> io::Result<u16> {
        Ok(self.local_address()?.port())
    }

    pub async fn send_to(&self, bytes: &[u8], peer: impl Into<SocketAddr>) -> io::Result<usize> {
        let peer = peer.into();
        match (self.selection, &self.sender, peer) {
            (Some(link), Some(sender), SocketAddr::V6(peer)) => {
                sender.send(&self.udp, bytes, peer, link).await
            }
            (None, None, peer) => self.udp.send_to(bytes, peer).await,
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid selected IPv6 socket destination",
            )),
        }
    }

    /// Read the next datagram. In normal mode one the selected link doesn't
    /// take is read and refused ([`Inbound::OffLink`]); the caller reads on.
    pub async fn recv_from(&self, bytes: &mut [u8]) -> io::Result<Inbound> {
        let received = self.receiver.recv_from(&self.udp, bytes).await?;
        Ok(classify(self.selection, received))
    }
}

/// Whether the transport takes a datagram the socket read: in normal mode,
/// with `selection`, only one [`admits`] lets in, whose link-local peer then
/// gets the link's scope. In foreign mode, with none, every one.
fn classify(selection: Option<SelectedLink>, mut received: ReceivedDatagram) -> Inbound {
    if let Some(link) = selection {
        if !admits(link, &received) {
            return Inbound::OffLink;
        }
        if let SocketAddr::V6(peer) = &mut received.peer {
            if peer.ip().is_unicast_link_local() {
                peer.set_scope_id(link.index);
            }
        }
    }
    Inbound::Datagram(received)
}

/// What one read of a [`Bip6Socket`] returned.
#[derive(Debug)]
pub(super) enum Inbound {
    /// A datagram for this transport: in normal mode, one that arrived on
    /// the selected link and was sent to its address or a BACnet group.
    Datagram(ReceivedDatagram),
    /// In normal mode, a datagram that arrived on another interface or was
    /// sent to another address. The wildcard socket gets the datagrams for
    /// every address and interface at its port, so this is routine on a
    /// host with more than one, or with another B/IPv6 port at the same
    /// UDP port.
    OffLink,
}

fn foreign_bind_address(
    requested: Ipv6Addr,
    port: u16,
    bbmd: SocketAddrV6,
) -> io::Result<SocketAddrV6> {
    if !requested.is_unspecified() {
        return Ok(SocketAddrV6::new(requested, port, 0, 0));
    }
    // A UDP connect selects a route/source without sending a packet. Bind the
    // production socket to that exact source, so subsequent registration and
    // DBTN cannot silently advertise one address while using another.
    let probe = std::net::UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))?;
    probe.connect(bbmd)?;
    match probe.local_addr()? {
        SocketAddr::V6(mut address)
            if !address.ip().is_unspecified() && !address.ip().is_multicast() =>
        {
            address.set_port(port);
            Ok(address)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no concrete IPv6 source for configured BBMD",
        )),
    }
}

fn admits(link: SelectedLink, datagram: &ReceivedDatagram) -> bool {
    datagram.arrival_index == Some(link.index)
        && link.index != 0
        && matches!(datagram.destination, IpAddr::V6(ip) if ip == link.address
            || [BACNET_IPV6_MULTICAST_LINK_LOCAL, BACNET_IPV6_MULTICAST_SITE_LOCAL,
                BACNET_IPV6_MULTICAST_ORG_LOCAL].contains(&ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_an_explicit_port_opts_into_address_reuse() {
        assert!(!udp_socket(0).unwrap().reuse_address().unwrap());
        assert!(udp_socket(0xBAC0).unwrap().reuse_address().unwrap());
        let ephemeral = Bip6Socket::bind(Ipv6Addr::LOCALHOST, 0, None)
            .await
            .unwrap();
        assert!(!socket2::SockRef::from(&ephemeral.udp)
            .reuse_address()
            .unwrap());
    }

    #[test]
    fn arrival_interface_and_selected_destination_are_required_before_learning() {
        let link = SelectedLink {
            address: "fd12::1".parse().unwrap(),
            index: 3,
        };
        let mut datagram = ReceivedDatagram {
            len: 11,
            peer: "[fd12::2]:47808".parse().unwrap(),
            destination: IpAddr::V6(BACNET_IPV6_MULTICAST_SITE_LOCAL),
            arrival_index: Some(3),
            os_group_delivery: None,
        };
        assert!(admits(link, &datagram));
        for index in [None, Some(0), Some(4)] {
            datagram.arrival_index = index;
            assert!(!admits(link, &datagram));
        }
        datagram.arrival_index = Some(3);
        datagram.destination = IpAddr::V6(link.address);
        assert!(admits(link, &datagram));
        for destination in ["fd12::9", "::1", "::", "ff05::1"] {
            datagram.destination = IpAddr::V6(destination.parse().unwrap());
            assert!(!admits(link, &datagram));
        }
    }

    /// In normal mode a datagram the selected link doesn't admit is read as
    /// off-link, and one it admits is kept, a link-local peer taking the
    /// link's scope. In foreign mode every datagram is kept as it came.
    #[test]
    fn a_datagram_for_another_link_or_address_is_off_link() {
        let link = SelectedLink {
            address: "fd12::1".parse().unwrap(),
            index: 3,
        };
        let datagram = |peer: &str, destination: &str, arrival_index| ReceivedDatagram {
            len: 11,
            peer: peer.parse().unwrap(),
            destination: destination.parse().unwrap(),
            arrival_index,
            os_group_delivery: None,
        };
        let elsewhere = [
            datagram("[fd12::2]:47808", "fd12::9", Some(3)),
            datagram("[fd12::2]:47808", "fd12::1", Some(4)),
            datagram("[fe80::2]:47808", "ff05::bac0", None),
        ];
        for received in elsewhere {
            assert!(
                matches!(classify(Some(link), received), Inbound::OffLink),
                "{received:?}"
            );
            assert!(matches!(
                classify(None, received),
                Inbound::Datagram(kept) if kept == received
            ));
        }

        let on_link = datagram("[fe80::2]:47808", "ff05::bac0", Some(3));
        let Inbound::Datagram(kept) = classify(Some(link), on_link) else {
            panic!("an admitted datagram is kept");
        };
        assert_eq!(kept.peer, "[fe80::2%3]:47808".parse().unwrap());
        assert_eq!((kept.destination, kept.len), (on_link.destination, 11));
        let unicast = datagram("[fd12::2]:47808", "fd12::1", Some(3));
        assert!(matches!(
            classify(Some(link), unicast),
            Inbound::Datagram(kept) if kept == unicast
        ));
    }
}
