//! The Winsock control-message parser, as plain safe code over bytes.
//!
//! It reads the layout from portable mirrors of `CMSGHDR`, `IN_PKTINFO` and
//! `IN6_PKTINFO`, so it builds and is tested on every host. On Windows, the
//! const checks below pin each mirror to windows-sys, field by field.

use std::mem::{offset_of, size_of};

use super::*;

/// `CMSGHDR`: the message length, then its level and type.
#[repr(C)]
pub(super) struct CmsgHeader {
    pub cmsg_len: usize,
    pub cmsg_level: i32,
    pub cmsg_type: i32,
}

/// `IN_PKTINFO`: the destination address and the arrival interface.
#[repr(C)]
pub(super) struct InPacketInfo {
    pub ipi_addr: [u8; 4],
    pub ipi_ifindex: u32,
}

/// `IN6_PKTINFO`: the destination address and the arrival interface.
#[repr(C)]
pub(super) struct In6PacketInfo {
    pub ipi6_addr: [u8; 16],
    pub ipi6_ifindex: u32,
}

/// Winsock's `IPPROTO_IP`, `IP_PKTINFO`, `IPPROTO_IPV6` and `IPV6_PKTINFO`.
pub(super) const IPPROTO_IP: i32 = 0;
pub(super) const IP_PKTINFO: i32 = 19;
pub(super) const IPPROTO_IPV6: i32 = 41;
pub(super) const IPV6_PKTINFO: i32 = 19;

#[cfg(windows)]
const _: () = {
    use windows_sys::Win32::Networking::WinSock as ws;
    assert!(size_of::<CmsgHeader>() == size_of::<ws::CMSGHDR>());
    assert!(align_of::<CmsgHeader>() == align_of::<ws::CMSGHDR>());
    assert!(align_of::<InPacketInfo>() == align_of::<ws::IN_PKTINFO>());
    assert!(align_of::<In6PacketInfo>() == align_of::<ws::IN6_PKTINFO>());
    assert!(offset_of!(CmsgHeader, cmsg_len) == offset_of!(ws::CMSGHDR, cmsg_len));
    assert!(offset_of!(CmsgHeader, cmsg_level) == offset_of!(ws::CMSGHDR, cmsg_level));
    assert!(offset_of!(CmsgHeader, cmsg_type) == offset_of!(ws::CMSGHDR, cmsg_type));
    assert!(size_of::<InPacketInfo>() == size_of::<ws::IN_PKTINFO>());
    assert!(offset_of!(InPacketInfo, ipi_addr) == offset_of!(ws::IN_PKTINFO, ipi_addr));
    assert!(offset_of!(InPacketInfo, ipi_ifindex) == offset_of!(ws::IN_PKTINFO, ipi_ifindex));
    assert!(size_of::<In6PacketInfo>() == size_of::<ws::IN6_PKTINFO>());
    assert!(offset_of!(In6PacketInfo, ipi6_addr) == offset_of!(ws::IN6_PKTINFO, ipi6_addr));
    assert!(offset_of!(In6PacketInfo, ipi6_ifindex) == offset_of!(ws::IN6_PKTINFO, ipi6_ifindex));
    assert!(IPPROTO_IP == ws::IPPROTO_IP);
    assert!(IP_PKTINFO == ws::IP_PKTINFO);
    assert!(IPPROTO_IPV6 == ws::IPPROTO_IPV6);
    assert!(IPV6_PKTINFO == ws::IPV6_PKTINFO);
};

/// The offset of a message's data: its header, padded to `usize`.
pub(super) const DATA_OFFSET: usize = size_of::<CmsgHeader>().next_multiple_of(size_of::<usize>());

/// The destination and arrival interface in a Winsock control buffer,
/// parsed from its bytes: every read is bounds-checked against `control`.
/// A message too short for its packet info is skipped, as before.
pub(super) fn windows_destination(
    control: &[u8],
    version: IpVersion,
) -> io::Result<(IpAddr, Option<u32>)> {
    let invalid = || invalid_metadata("invalid Windows UDP control message");
    let mut rest = control;
    let mut destination = None;
    while rest.len() >= size_of::<CmsgHeader>() {
        let field = |offset| octets_at::<4>(rest, offset).map(i32::from_ne_bytes);
        let length = octets_at(rest, offset_of!(CmsgHeader, cmsg_len)).map(usize::from_ne_bytes);
        let level = field(offset_of!(CmsgHeader, cmsg_level));
        let kind = field(offset_of!(CmsgHeader, cmsg_type));
        let (Some(length), Some(level), Some(kind)) = (length, level, kind) else {
            return Err(invalid());
        };
        // The message must hold its header and lie whole inside the buffer.
        if length < DATA_OFFSET || length > rest.len() {
            return Err(invalid());
        }
        let data = &rest[DATA_OFFSET..length];
        let candidate = match version {
            IpVersion::V4 if level == IPPROTO_IP && kind == IP_PKTINFO => {
                let address = octets_at::<4>(data, offset_of!(InPacketInfo, ipi_addr));
                let index = octets_at(data, offset_of!(InPacketInfo, ipi_ifindex));
                address
                    .zip(index)
                    .filter(|_| data.len() >= size_of::<InPacketInfo>())
                    .map(|(address, index)| {
                        (
                            IpAddr::V4(Ipv4Addr::from(address)),
                            Some(u32::from_ne_bytes(index)),
                        )
                    })
            }
            IpVersion::V6 if level == IPPROTO_IPV6 && kind == IPV6_PKTINFO => {
                let address = octets_at::<16>(data, offset_of!(In6PacketInfo, ipi6_addr));
                let index = octets_at(data, offset_of!(In6PacketInfo, ipi6_ifindex));
                address
                    .zip(index)
                    .filter(|_| data.len() >= size_of::<In6PacketInfo>())
                    .map(|(address, index)| {
                        (
                            IpAddr::V6(Ipv6Addr::from(address)),
                            Some(u32::from_ne_bytes(index)),
                        )
                    })
            }
            _ => None,
        };
        if let Some(candidate) = candidate
            && destination.replace(candidate).is_some()
        {
            return Err(invalid_metadata("duplicate UDP destination metadata"));
        }
        let next = length.next_multiple_of(size_of::<usize>());
        rest = rest.get(next..).unwrap_or_default();
    }
    destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))
}

#[cfg(test)]
#[path = "udp_metadata_winsock_tests.rs"]
mod tests;
