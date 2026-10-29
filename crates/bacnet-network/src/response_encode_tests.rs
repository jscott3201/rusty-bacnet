//! The reply NPDU encoder and the header size the APDU budget subtracts
//! (#1713) match the owned NPDU encoder octet for octet, and refuse what it
//! refuses with its error.
use super::{ResponseRoute, encode_checked_response_npdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_npdu::NetworkPriority;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};

fn owned(
    apdu: &[u8],
    destination: Option<&NpduAddress>,
    expecting_reply: bool,
    priority: NetworkPriority,
) -> Result<BytesMut, String> {
    let npdu = Npdu {
        destination: destination.cloned(),
        expecting_reply,
        priority,
        payload: Bytes::copy_from_slice(apdu),
        ..Npdu::default()
    };
    let mut encoded = BytesMut::new();
    encode_npdu(&mut encoded, &npdu).map_err(|error| error.to_string())?;
    Ok(encoded)
}

fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

fn destinations() -> Vec<Option<NpduAddress>> {
    vec![
        None,
        Some(address(5, &[3])),
        Some(address(0xfffe, &[1, 2, 3, 4, 5, 6])),
        // A remote broadcast: a DNET with no DADR.
        Some(address(7, &[])),
        Some(address(9, &[0xaa; 18])),
    ]
}

#[test]
fn reply_npdu_matches_the_owned_encoder() {
    let apdus: [&[u8]; 3] = [&[], &[0x30, 0x01, 0x0c], &[0x55; 600]];
    for destination in destinations() {
        for apdu in apdus {
            for expecting_reply in [false, true] {
                for priority in [NetworkPriority::Normal, NetworkPriority::LifeSafety] {
                    let fast = encode_checked_response_npdu(
                        apdu,
                        destination.as_ref(),
                        expecting_reply,
                        priority,
                    )
                    .unwrap();
                    let slow = owned(apdu, destination.as_ref(), expecting_reply, priority);
                    assert_eq!(fast, slow.unwrap(), "{destination:?}");
                }
            }
        }
    }
}

#[test]
fn refused_reply_npdu_reports_the_owned_encoder_error() {
    let long = address(9, &[0xaa; 19]);
    let fast = encode_checked_response_npdu(&[1], Some(&long), false, NetworkPriority::Normal)
        .unwrap_err()
        .to_string();
    let slow = owned(&[1], Some(&long), false, NetworkPriority::Normal).unwrap_err();
    assert_eq!(fast, slow);
}

/// The header length of a route with no router; `budget_tests` checks a
/// router route's budget against its port's NPDU cap.
#[test]
fn apdu_budget_header_is_the_header_the_reply_carries() {
    let route = ResponseRoute::unverified();
    for destination in destinations() {
        let header = owned(&[], destination.as_ref(), false, NetworkPriority::Normal)
            .unwrap()
            .len();
        assert_eq!(route.header_len(destination.as_ref()).unwrap(), header);
    }
    let long = address(9, &[0xaa; 19]);
    assert!(route.header_len(Some(&long)).is_err());
    assert!(route.max_apdu_length(480, Some(&long)).is_err());
    for network in [0, 0xffff] {
        assert!(
            route
                .max_apdu_length(480, Some(&address(network, &[1])))
                .is_err()
        );
    }
}
