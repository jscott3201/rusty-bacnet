use super::*;

/// One Winsock control message of `level` and `kind` around `payload`,
/// padded to `usize` as the next header would be.
fn message(level: i32, kind: i32, payload: &[u8]) -> Vec<u8> {
    let length = DATA_OFFSET + payload.len();
    let mut bytes = vec![0; length.next_multiple_of(size_of::<usize>())];
    let at = offset_of!(CmsgHeader, cmsg_len);
    bytes[at..at + size_of::<usize>()].copy_from_slice(&length.to_ne_bytes());
    let at = offset_of!(CmsgHeader, cmsg_level);
    bytes[at..at + 4].copy_from_slice(&level.to_ne_bytes());
    let at = offset_of!(CmsgHeader, cmsg_type);
    bytes[at..at + 4].copy_from_slice(&kind.to_ne_bytes());
    bytes[DATA_OFFSET..length].copy_from_slice(payload);
    bytes
}

fn ipv4_info(address: Ipv4Addr, index: u32) -> Vec<u8> {
    let mut bytes = vec![0; size_of::<InPacketInfo>()];
    let at = offset_of!(InPacketInfo, ipi_addr);
    bytes[at..at + 4].copy_from_slice(&address.octets());
    let at = offset_of!(InPacketInfo, ipi_ifindex);
    bytes[at..at + 4].copy_from_slice(&index.to_ne_bytes());
    message(IPPROTO_IP, IP_PKTINFO, &bytes)
}

fn ipv6_info(address: Ipv6Addr, index: u32) -> Vec<u8> {
    let mut bytes = vec![0; size_of::<In6PacketInfo>()];
    let at = offset_of!(In6PacketInfo, ipi6_addr);
    bytes[at..at + 16].copy_from_slice(&address.octets());
    let at = offset_of!(In6PacketInfo, ipi6_ifindex);
    bytes[at..at + 4].copy_from_slice(&index.to_ne_bytes());
    message(IPPROTO_IPV6, IPV6_PKTINFO, &bytes)
}

fn kind_of(result: io::Result<(IpAddr, Option<u32>)>) -> io::ErrorKind {
    result.unwrap_err().kind()
}

#[test]
fn the_data_offset_matches_winsock_alignment() {
    // WSA_CMSGDATA_ALIGN pads the header to the pointer size. Pinned here
    // because the other tests build messages from DATA_OFFSET itself.
    let expected = if cfg!(target_pointer_width = "64") {
        16
    } else {
        12
    };
    assert_eq!(DATA_OFFSET, expected);
}

#[test]
fn packet_info_is_read_for_either_family() {
    let v4 = Ipv4Addr::new(192, 168, 1, 7);
    assert_eq!(
        windows_destination(&ipv4_info(v4, 3), IpVersion::V4).unwrap(),
        (IpAddr::V4(v4), Some(3))
    );
    let v6: Ipv6Addr = "fd12::1".parse().unwrap();
    // Another message first: it is stepped over by its padded length.
    let mut control = message(IPPROTO_IP, 99, &[1, 2, 3]);
    control.extend(ipv6_info(v6, 9));
    assert_eq!(
        windows_destination(&control, IpVersion::V6).unwrap(),
        (IpAddr::V6(v6), Some(9))
    );
}

#[test]
fn a_header_cut_short_or_running_past_the_buffer_is_invalid() {
    let whole = ipv4_info(Ipv4Addr::LOCALHOST, 1);
    // Leftover octets too few for a header end the walk: no destination.
    let short_header = &whole[..size_of::<CmsgHeader>() - 1];
    assert_eq!(
        kind_of(windows_destination(short_header, IpVersion::V4)),
        io::ErrorKind::InvalidData
    );
    // A length past the end of the buffer.
    let mut past = whole.clone();
    let at = offset_of!(CmsgHeader, cmsg_len);
    past[at..at + size_of::<usize>()].copy_from_slice(&(whole.len() + 1).to_ne_bytes());
    assert_eq!(
        kind_of(windows_destination(&past, IpVersion::V4)),
        io::ErrorKind::InvalidData
    );
    // A length shorter than the header itself.
    let mut under = whole;
    under[at..at + size_of::<usize>()].copy_from_slice(&(DATA_OFFSET - 1).to_ne_bytes());
    assert_eq!(
        kind_of(windows_destination(&under, IpVersion::V4)),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn a_short_payload_is_skipped_and_leaves_the_destination_missing() {
    let short = message(IPPROTO_IPV6, IPV6_PKTINFO, &[0; 19]);
    assert_eq!(
        kind_of(windows_destination(&short, IpVersion::V6)),
        io::ErrorKind::InvalidData
    );
    // Skipped, not fatal: a whole one after it still reads.
    let mut control = short;
    control.extend(ipv6_info(Ipv6Addr::LOCALHOST, 2));
    assert_eq!(
        windows_destination(&control, IpVersion::V6).unwrap(),
        (IpAddr::V6(Ipv6Addr::LOCALHOST), Some(2))
    );
}

#[test]
fn duplicate_or_missing_packet_info_is_invalid() {
    let mut v4 = ipv4_info(Ipv4Addr::LOCALHOST, 1);
    v4.extend(ipv4_info(Ipv4Addr::LOCALHOST, 1));
    assert_eq!(
        kind_of(windows_destination(&v4, IpVersion::V4)),
        io::ErrorKind::InvalidData
    );
    let mut v6 = ipv6_info(Ipv6Addr::LOCALHOST, 1);
    v6.extend(ipv6_info(Ipv6Addr::LOCALHOST, 1));
    assert_eq!(
        kind_of(windows_destination(&v6, IpVersion::V6)),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        kind_of(windows_destination(&[], IpVersion::V4)),
        io::ErrorKind::InvalidData
    );
    // The other family's packet info is not this one's.
    assert_eq!(
        kind_of(windows_destination(
            &ipv4_info(Ipv4Addr::LOCALHOST, 1),
            IpVersion::V6
        )),
        io::ErrorKind::InvalidData
    );
}
