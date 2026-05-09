use super::*;
use crate::port::TransportPort;

#[test]
fn encode_decode_round_trip() {
    let dst = ETHERNET_BROADCAST;
    let src = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    let npdu = vec![0x01, 0x00, 0xAA, 0xBB];

    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);

    let decoded = decode_ethernet_frame(&buf).unwrap();
    assert_eq!(decoded.destination, dst);
    assert_eq!(decoded.source, src);
    assert_eq!(decoded.payload, npdu);
}

#[test]
fn llc_header_correct() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    let npdu = vec![0xAA];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    assert_eq!(buf[14], BACNET_LLC_DSAP);
    assert_eq!(buf[15], BACNET_LLC_SSAP);
    assert_eq!(buf[16], LLC_CONTROL_UI);
}

#[test]
fn length_field_correct() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    let npdu = vec![0x01, 0x02, 0x03];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    let length = u16::from_be_bytes([buf[12], buf[13]]);
    assert_eq!(length as usize, LLC_HEADER_LEN + npdu.len());
}

#[test]
fn rejects_short_frame() {
    assert!(decode_ethernet_frame(&[0; 10]).is_err());
}

#[test]
fn rejects_invalid_llc() {
    let mut buf = vec![0u8; 20];
    buf[14] = 0x00; // wrong DSAP
    buf[15] = 0x82;
    buf[16] = 0x03;
    buf[12] = 0x00;
    buf[13] = 0x04; // length = 4
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn rejects_truncated_payload() {
    let mut buf = vec![0u8; MIN_FRAME_LEN];
    buf[14] = BACNET_LLC_DSAP;
    buf[15] = BACNET_LLC_SSAP;
    buf[16] = LLC_CONTROL_UI;
    // Length claims more data than available
    buf[12] = 0x00;
    buf[13] = 0xFF;
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn rejects_ethertype_as_length() {
    let mut buf = vec![0u8; 20];
    buf[12] = 0x08;
    buf[13] = 0x00; // length = 2048 > 1500
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn rejects_length_1501() {
    let mut buf = vec![0u8; 20];
    buf[12] = (1501u16 >> 8) as u8;
    buf[13] = (1501u16 & 0xFF) as u8;
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn accepts_length_1500() {
    let mut buf = vec![0u8; 14 + 1500];
    buf[12] = (1500u16 >> 8) as u8;
    buf[13] = (1500u16 & 0xFF) as u8;
    buf[14] = BACNET_LLC_DSAP;
    buf[15] = BACNET_LLC_SSAP;
    buf[16] = LLC_CONTROL_UI;
    let result = decode_ethernet_frame(&buf);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().payload.len(), 1500 - LLC_HEADER_LEN);
}

#[test]
fn encode_pads_small_frame_to_minimum() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    // 1-byte NPDU: frame would be 14 + 3 + 1 = 18 bytes without padding
    let npdu = vec![0xAA];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    // Must be padded to 60 bytes (14 header + 46 payload)
    assert_eq!(buf.len(), 60);
    // Verify padding is zeros
    for &b in &buf[18..60] {
        assert_eq!(b, 0x00);
    }
}

#[test]
fn encode_does_not_pad_large_frame() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    // 50-byte NPDU: frame is 14 + 3 + 50 = 67 bytes, above 60-byte minimum
    let npdu = vec![0xBB; 50];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    assert_eq!(buf.len(), 67); // no padding needed
}

#[test]
fn padded_frame_decodes_correctly() {
    let dst = [0xFF; 6];
    let src = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    let npdu = vec![0x01]; // tiny payload
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    // Should be padded to 60 bytes
    assert_eq!(buf.len(), 60);
    // Decode should extract only the declared payload (not padding)
    let decoded = decode_ethernet_frame(&buf).unwrap();
    assert_eq!(decoded.payload, npdu);
    assert_eq!(decoded.source, src);
}

#[cfg(target_os = "linux")]
#[test]
fn ethernet_transport_new() {
    let t = EthernetTransport::new("eth0");
    assert_eq!(t.local_mac(), &[0; 6]); // not started yet
}
