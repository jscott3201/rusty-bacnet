//! A confirmed request whose link-layer source is a group address is
//! ignored and counted (#1504): the acknowledgment of a confirmed COV
//! notification would otherwise go back to the group, to every node in it.
//! The built-in transports hand up no such source, so a custom link does.

use super::*;
use bacnet_encoding::apdu::{decode_apdu, ConfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

/// A link that hands up what the test feeds it, from any source MAC, reports
/// each send with its destination, and counts MAC 0xFF as a group.
struct GroupSourceLink {
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    sends: mpsc::UnboundedSender<(MacAddr, Bytes)>,
}

impl TransportPort for GroupSourceLink {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound_rx.take().expect("transport started once"))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let sent = (MacAddr::from_slice(mac), Bytes::copy_from_slice(npdu));
        let _ = self.sends.send(sent);
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &[]).await
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[0x01]
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        mac == [0xFF]
    }
}

/// A ConfirmedCOVNotification for `process_id` from link-layer `source`,
/// with `process_id` as its invoke ID too.
fn notification_from(source: u8, process_id: u32) -> ReceivedNpdu {
    let mut body = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: process_id,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        time_remaining: 60,
        list_of_values: vec![bacnet_services::common::BACnetPropertyValue {
            property_identifier: bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut body);
    let request = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: process_id as u8,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        service_request: body.freeze(),
    };
    let mut apdu = BytesMut::new();
    encode_apdu(&mut apdu, &Apdu::ConfirmedRequest(request)).unwrap();
    let mut npdu = BytesMut::new();
    let fields = Npdu {
        expecting_reply: true,
        payload: apdu.freeze(),
        ..Npdu::default()
    };
    encode_npdu(&mut npdu, &fields).unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&[source]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

#[tokio::test]
async fn a_confirmed_request_from_a_group_source_is_ignored_and_counted() {
    let (inbound_tx, inbound_rx) = mpsc::channel(8);
    let (sends, mut sent) = mpsc::unbounded_channel();
    let link = GroupSourceLink {
        inbound_rx: Some(inbound_rx),
        sends,
    };
    let mut client = BACnetClient::start(ClientConfig::default(), link)
        .await
        .unwrap();
    let mut notifications = client.cov_notifications();
    inbound_tx.send(notification_from(0xFF, 1)).await.unwrap();
    inbound_tx.send(notification_from(0x02, 2)).await.unwrap();

    // The peer's notification is the first delivered and the first acked.
    let delivered = timeout(Duration::from_secs(2), notifications.recv())
        .await
        .expect("the peer's notification is delivered")
        .unwrap();
    assert_eq!(delivered.notification.subscriber_process_identifier, 2);
    let (to, npdu) = timeout(Duration::from_secs(2), sent.recv())
        .await
        .expect("the peer's notification is acknowledged")
        .unwrap();
    assert_eq!(to[..], [0x02]);
    let apdu = decode_apdu(decode_npdu(npdu).unwrap().payload).unwrap();
    let Apdu::SimpleAck(ack) = apdu else {
        panic!("expected SimpleAck, got {apdu:?}");
    };
    assert_eq!(ack.invoke_id, 2);
    assert_eq!(client.group_source_request_drops(), 1);
    client.stop().await.unwrap();
    assert!(sent.try_recv().is_err(), "nothing went to the group");
}
