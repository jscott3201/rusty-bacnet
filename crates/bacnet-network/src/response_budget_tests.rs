//! The APDU budget of a reply through a router route (#1713): the port's
//! NPDU cap less the reply's real header, from the allocation-free header
//! length. A direct capability can't be built in this crate; bacnet-server's
//! `direct_principal_tests::responses` budget tests pin that arm, and fail
//! when the header length is wrong.
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_npdu::NpduLimits;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::router::{BACnetRouter, RouterOptions, RouterPort};

/// The cap each port reports, well under the largest NPDU.
const PORT_NPDU: u16 = 600;

fn port(mac: u8) -> (TestTransport, mpsc::Sender<ReceivedNpdu>) {
    let (frames, incoming) = mpsc::channel(8);
    let port = TestTransport::builder()
        .local_mac(&[mac])
        .inbound(incoming)
        .receive_capacity(480)
        .max_npdu(NpduLimits::both(PORT_NPDU))
        .build();
    (port, frames)
}

fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

#[tokio::test]
async fn router_route_budget_is_the_port_cap_less_the_reply_header() {
    let (a, frames) = port(1);
    let (b, _frames_b) = port(2);
    let mut started = BACnetRouter::start(
        vec![
            RouterPort {
                transport: a,
                network_number: 1000,
            },
            RouterPort {
                transport: b,
                network_number: 2000,
            },
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    // A request relayed from network 9000 to this router's station on 2000.
    let request = Npdu {
        destination: Some(address(2000, &[2])),
        source: Some(address(9000, &[3, 4])),
        expecting_reply: true,
        payload: Bytes::from_static(&[0, 5, 0x37, 12]),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &request).unwrap();
    frames
        .send(ReceivedNpdu::unverified(
            bytes.freeze(),
            MacAddr::from_slice(&[9]),
            false,
            vec![],
            None,
        ))
        .await
        .unwrap();
    let received = started.apdus.recv().await.unwrap();
    let route = received.response_route();
    // The reply's NPCI: version and control, DNET 9000 with its two-octet
    // DADR, SNET 2000 with its one-octet SADR, and the hop count.
    let header = 2 + (3 + 2) + (3 + 1) + 1;
    assert_eq!(route.header_len(None).unwrap(), header);
    let cap = usize::from(PORT_NPDU) - header;
    assert_eq!(
        usize::from(route.max_apdu_length(u16::MAX, None).unwrap()),
        cap
    );
    assert_eq!(usize::from(route.max_apdu_length(1000, None).unwrap()), cap);
    // A smaller APDU limit wins.
    assert_eq!(route.max_apdu_length(480, None).unwrap(), 480);
    started.router.stop().await.unwrap();
}
