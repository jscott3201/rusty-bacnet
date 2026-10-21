use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::write_group::GroupChannelValue;
use bacnet_transport::port::ReceivedNpdu;
use std::num::NonZeroU32;

const PEER: [u8; 6] = [10, 0, 0, 5, 0xBA, 0xC0];

/// One frame the client sent: its NPDU, and the link address it went to, or
/// `None` for a link broadcast.
struct Sent {
    npdu: Bytes,
    to: Option<MacAddr>,
}

/// Records every frame instead of sending it.
struct Capture {
    inbound: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent: mpsc::UnboundedSender<Sent>,
}

impl TransportPort for Capture {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inbound
            .take()
            .ok_or_else(|| Error::Encoding("capture transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let _ = self.sent.send(Sent {
            npdu: Bytes::copy_from_slice(npdu),
            to: Some(MacAddr::from_slice(mac)),
        });
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let _ = self.sent.send(Sent {
            npdu: Bytes::copy_from_slice(npdu),
            to: None,
        });
        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[10, 0, 0, 2, 0xBA, 0xC0]
    }
}

async fn client() -> (
    BACnetClient<Capture>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<Sent>,
) {
    let (inbound_tx, inbound) = mpsc::channel(4);
    let (sent, sent_rx) = mpsc::unbounded_channel();
    let transport = Capture {
        inbound: Some(inbound),
        sent,
    };
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    (client, inbound_tx, sent_rx)
}

/// Annex F.3.11's second example: group 23 at priority 8, channel 12 to
/// REAL 67.0 and channel 13 to REAL 72.0, with Inhibit Delay TRUE.
fn request() -> WriteGroupRequest {
    let value = |real: f32| {
        let mut bytes = vec![0x44];
        bytes.extend_from_slice(&real.to_be_bytes());
        bytes
    };
    WriteGroupRequest {
        group_number: NonZeroU32::new(23).unwrap(),
        write_priority: 8,
        change_list: vec![
            GroupChannelValue {
                channel: 12,
                override_priority: None,
                value: value(67.0),
            },
            GroupChannelValue {
                channel: 13,
                override_priority: None,
                value: value(72.0),
            },
        ],
        inhibit_delay: Some(true),
    }
}

/// The example's APDU: Unconfirmed-Request, service choice 10, the request.
const APDU: [u8; 24] = [
    0x10, 0x0A, 0x09, 0x17, 0x19, 0x08, 0x2E, 0x09, 0x0C, 0x44, 0x42, 0x86, 0x00, 0x00, 0x09, 0x0D,
    0x44, 0x42, 0x90, 0x00, 0x00, 0x2F, 0x39, 0x01,
];

#[tokio::test]
async fn write_group_sends_the_request_to_a_device_or_a_broadcast() {
    let (mut client, _inbound, mut sent) = client().await;
    let cases = [
        (
            WriteGroupDestination::Device(MacAddr::from_slice(&PEER)),
            Some(&PEER[..]),
            None,
        ),
        (WriteGroupDestination::LocalBroadcast, None, None),
        (WriteGroupDestination::RemoteBroadcast(100), None, Some(100)),
        (
            WriteGroupDestination::RemoteBroadcast(65534),
            None,
            Some(65534),
        ),
        (WriteGroupDestination::GlobalBroadcast, None, Some(u16::MAX)),
    ];
    for (destination, to, network) in cases {
        client.write_group(&destination, &request()).await.unwrap();
        let frame = sent.try_recv().expect("one frame per request");
        assert_eq!(frame.to.as_deref(), to, "{destination:?}");
        let npdu = decode_npdu(frame.npdu).unwrap();
        assert!(!npdu.expecting_reply, "{destination:?}");
        assert_eq!(npdu.priority, NetworkPriority::Normal);
        match network {
            Some(network) => {
                let dnet = npdu.destination.expect("a DNET");
                assert_eq!(dnet.network, network);
                assert!(dnet.mac_address.is_empty(), "{destination:?}");
            }
            None => assert!(npdu.destination.is_none(), "{destination:?}"),
        }
        assert_eq!(npdu.payload[..], APDU, "{destination:?}");
        assert!(sent.try_recv().is_err());
    }
    client.stop().await.unwrap();
}

#[tokio::test]
async fn write_group_refuses_a_bad_request_or_network_before_sending() {
    let (mut client, _inbound, mut sent) = client().await;
    for network in [0, u16::MAX] {
        let result = client
            .write_group(&WriteGroupDestination::RemoteBroadcast(network), &request())
            .await;
        assert!(
            matches!(result, Err(Error::Encoding(_))),
            "{network}: {result:?}"
        );
    }
    let mut bad = request();
    bad.write_priority = 17;
    let result = client
        .write_group(&WriteGroupDestination::LocalBroadcast, &bad)
        .await;
    assert!(matches!(result, Err(Error::Encode(_))), "{result:?}");
    bad = request();
    bad.change_list.clear();
    let result = client
        .write_group(&WriteGroupDestination::GlobalBroadcast, &bad)
        .await;
    assert!(matches!(result, Err(Error::Encode(_))), "{result:?}");
    assert!(sent.try_recv().is_err(), "a refused request went out");
    client.stop().await.unwrap();
}
