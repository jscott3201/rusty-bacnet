use std::net::{Ipv6Addr, SocketAddrV6};

use bacnet_types::error::Error;
use bytes::{BufMut, Bytes, BytesMut};

use super::{
    Bip6Vmac, Bvlc6Frame, Bvlc6Function, BVLC6_HEADER_LENGTH, BVLC6_TYPE,
    BVLC6_UNICAST_HEADER_LENGTH,
};

/// Encode a BVLC-IPv6 frame into a buffer.
pub fn encode_bvlc6(
    buf: &mut BytesMut,
    function: Bvlc6Function,
    source_vmac: &Bip6Vmac,
    npdu: &[u8],
) -> Result<(), Error> {
    let total_length = BVLC6_HEADER_LENGTH + npdu.len();
    if total_length > u16::MAX as usize {
        return Err(Error::Encoding(format!(
            "BVLC6 frame length {total_length} exceeds 16-bit BVLC length field"
        )));
    }
    buf.reserve(total_length);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(function.to_byte());
    buf.put_u16(total_length as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(npdu);
    Ok(())
}

/// Decode a BVLC-IPv6 frame from raw bytes.
pub fn decode_bvlc6(data: &[u8]) -> Result<Bvlc6Frame, Error> {
    if data.len() < BVLC6_HEADER_LENGTH {
        return Err(Error::decoding(
            0,
            format!(
                "BVLC6 frame too short: need {} bytes, have {}",
                BVLC6_HEADER_LENGTH,
                data.len()
            ),
        ));
    }

    if data[0] != BVLC6_TYPE {
        return Err(Error::decoding(
            0,
            format!("BVLC6 expected type 0x82, got 0x{:02X}", data[0]),
        ));
    }

    let function = Bvlc6Function::from_byte(data[1]);
    let length = u16::from_be_bytes([data[2], data[3]]) as usize;

    if length < BVLC6_HEADER_LENGTH {
        return Err(Error::decoding(2, "BVLC6 length less than header size"));
    }
    if length > data.len() {
        return Err(Error::decoding(
            2,
            format!("BVLC6 length {} exceeds data length {}", length, data.len()),
        ));
    }

    let mut source_vmac = [0u8; 3];
    source_vmac.copy_from_slice(&data[4..7]);

    // These message types include a 3-byte destination/target VMAC after the source VMAC
    let has_dest_vmac = matches!(
        function,
        Bvlc6Function::OriginalUnicast
            | Bvlc6Function::AddressResolution
            | Bvlc6Function::AddressResolutionAck
            | Bvlc6Function::VirtualAddressResolutionAck
    );

    let (destination_vmac, payload_start) = if has_dest_vmac {
        if length < BVLC6_UNICAST_HEADER_LENGTH {
            return Err(Error::decoding(
                7,
                "BVLC6 frame too short for destination VMAC",
            ));
        }
        let mut dest = [0u8; 3];
        dest.copy_from_slice(&data[7..10]);
        (Some(dest), BVLC6_UNICAST_HEADER_LENGTH)
    } else {
        (None, BVLC6_HEADER_LENGTH)
    };

    let payload = Bytes::copy_from_slice(&data[payload_start..length]);

    Ok(Bvlc6Frame {
        function,
        source_vmac,
        destination_vmac,
        payload,
    })
}

/// Encode a BVLC-IPv6 Original-Unicast-NPDU frame.
pub fn encode_bvlc6_original_unicast(
    buf: &mut BytesMut,
    source_vmac: &Bip6Vmac,
    dest_vmac: &Bip6Vmac,
    npdu: &[u8],
) -> Result<(), Error> {
    let total_length = BVLC6_UNICAST_HEADER_LENGTH + npdu.len();
    if total_length > u16::MAX as usize {
        return Err(Error::Encoding(format!(
            "BVLC6 Original-Unicast-NPDU length {total_length} exceeds 16-bit BVLC length field"
        )));
    }
    buf.reserve(total_length);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::OriginalUnicast.to_byte());
    buf.put_u16(total_length as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf.put_slice(npdu);
    Ok(())
}

/// Encode a BVLC-IPv6 Original-Broadcast-NPDU frame.
pub fn encode_bvlc6_original_broadcast(
    buf: &mut BytesMut,
    source_vmac: &Bip6Vmac,
    npdu: &[u8],
) -> Result<(), Error> {
    encode_bvlc6(buf, Bvlc6Function::OriginalBroadcast, source_vmac, npdu)
}

/// Encode a BVLC-IPv6 Virtual-Address-Resolution frame (7 bytes, no payload).
///
/// Per spec Clause U.2.7: type(1) + function(1) + length(2) + source_vmac(3) = 7.
pub fn encode_virtual_address_resolution(source_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH);
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::VirtualAddressResolution,
        source_vmac,
        &[], // no payload
    )
    .expect("empty Virtual-Address-Resolution frame fits in BVLC6 length field");
    buf
}

/// Encode a BVLC-IPv6 Virtual-Address-Resolution-Ack frame (10 bytes).
///
/// Per spec Clause U.2.7A: includes the requester's VMAC as destination.
/// type(1) + function(1) + length(2) + source_vmac(3) + dest_vmac(3) = 10.
pub fn encode_virtual_address_resolution_ack(
    source_vmac: &Bip6Vmac,
    dest_vmac: &Bip6Vmac,
) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::VirtualAddressResolutionAck.to_byte());
    buf.put_u16(BVLC6_UNICAST_HEADER_LENGTH as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf
}

/// Encode a BVLC-IPv6 Address-Resolution frame (10 bytes).
///
/// Per spec Clause U.2.4: includes the target VMAC to resolve.
pub fn encode_address_resolution(source_vmac: &Bip6Vmac, target_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::AddressResolution.to_byte());
    buf.put_u16(BVLC6_UNICAST_HEADER_LENGTH as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(target_vmac);
    buf
}

/// Encode a BVLC-IPv6 Address-Resolution-Ack frame (10 bytes).
///
/// Per spec Clause U.2.5: includes the requester's VMAC as destination.
pub fn encode_address_resolution_ack(source_vmac: &Bip6Vmac, dest_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::AddressResolutionAck.to_byte());
    buf.put_u16(BVLC6_UNICAST_HEADER_LENGTH as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf
}

/// Extract the NPDU from a ForwardedNpdu payload.
///
/// ForwardedNpdu payload layout:
///   Original-Source-Virtual-Address(3) + Original-Source-B/IPv6-Address(18) + NPDU.
/// Returns the originating VMAC, originating B/IPv6 address, and NPDU bytes.
pub fn decode_forwarded_npdu_payload(
    payload: &[u8],
) -> Result<(Bip6Vmac, SocketAddrV6, &[u8]), Error> {
    if payload.len() < 21 {
        return Err(Error::decoding(
            0,
            format!(
                "ForwardedNpdu payload too short: need at least 21 bytes, have {}",
                payload.len()
            ),
        ));
    }
    let mut originating_vmac = [0u8; 3];
    originating_vmac.copy_from_slice(&payload[..3]);

    let mut ipv6_bytes = [0u8; 16];
    ipv6_bytes.copy_from_slice(&payload[3..19]);
    let ipv6_addr = Ipv6Addr::from(ipv6_bytes);
    let port = u16::from_be_bytes([payload[19], payload[20]]);
    let source_addr = SocketAddrV6::new(ipv6_addr, port, 0, 0);

    Ok((originating_vmac, source_addr, &payload[21..]))
}
