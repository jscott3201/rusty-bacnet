//! Independent OS packet-info oracle, used only on an explicit isolated link.
//! It reads IPV6_PKTINFO through nix rather than the transport's own
//! receive code, so the two can't share a fault.

use nix::sys::socket::{ControlMessageOwned, MsgFlags, SockaddrIn6, recvmsg, setsockopt, sockopt};
use std::{
    io,
    io::IoSliceMut,
    net::{Ipv6Addr, SocketAddrV6},
    os::fd::AsRawFd,
};
use tokio::{io::Interest, net::UdpSocket};

#[derive(Debug)]
pub(super) struct Frame {
    pub bytes: Vec<u8>,
    pub source: SocketAddrV6,
    pub destination: Ipv6Addr,
    pub index: u32,
}

pub(super) fn configure(socket: &socket2::Socket) {
    setsockopt(socket, sockopt::Ipv6RecvPacketInfo, &true).expect("packet-info oracle setup");
}

pub(super) async fn receive(socket: &UdpSocket) -> io::Result<Frame> {
    socket
        .async_io(Interest::READABLE, || {
            let mut bytes = [0u8; 2048];
            let mut control = nix::cmsg_space!(nix::libc::in6_pktinfo);
            let mut iov = [IoSliceMut::new(&mut bytes)];
            let message = recvmsg::<SockaddrIn6>(
                socket.as_raw_fd(),
                &mut iov,
                Some(&mut control),
                MsgFlags::empty(),
            )?;
            assert!(
                !message
                    .flags
                    .intersects(MsgFlags::MSG_TRUNC | MsgFlags::MSG_CTRUNC)
            );
            let peer = message.address.expect("wire oracle requires an IPv6 peer");
            let mut info = None;
            for control in message.cmsgs()? {
                if let ControlMessageOwned::Ipv6PacketInfo(packet) = control {
                    assert!(info.is_none(), "duplicate packet-info in wire oracle");
                    info = Some((
                        Ipv6Addr::from(packet.ipi6_addr.s6_addr),
                        packet.ipi6_ifindex,
                    ));
                }
            }
            let count = message.bytes;
            let (destination, index) =
                info.expect("wire oracle requires actual destination and arrival index");
            Ok(Frame {
                bytes: bytes[..count].to_vec(),
                source: SocketAddrV6::new(peer.ip(), peer.port(), 0, peer.scope_id()),
                destination,
                index,
            })
        })
        .await
}
