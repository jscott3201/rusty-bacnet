//! Wire harness and independent list decoder for the Device
//! `Active_COV_Subscriptions` projection tests.
use super::super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::{decode_object_property_reference, decode_recipient};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_encoding::{primitives, tags};
use bacnet_objects::analog::AnalogValueObject;
pub(super) use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{
    ReadAccessSpecification, ReadPropertyMultipleACK, ReadPropertyMultipleRequest,
};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
pub(super) use bacnet_types::constructed::{
    BACnetAddress, BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
    BACnetRecipientProcess,
};

const DEVICE_INSTANCE: u32 = 813;
pub(super) const ACTIVE: PropertyIdentifier = PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS;
pub(super) const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

pub(super) struct WireTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
}

impl TransportPort for WireTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.incoming.take().unwrap())
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[0x0A, 0, 0, 2, 0xBA, 0xC0]
    }
}

/// A subscriber as seen by the server: link source plus optional routed source.
#[derive(Clone)]
pub(super) struct Peer {
    pub(super) mac: Vec<u8>,
    pub(super) network: Option<NpduAddress>,
}

pub(super) fn direct() -> Peer {
    Peer {
        mac: vec![0x0A, 0, 0, 5, 0xBA, 0xC0],
        network: None,
    }
}

/// Behind a router whose link MAC differs from the remote subscriber MAC.
pub(super) fn routed() -> Peer {
    Peer {
        mac: vec![0x0A, 0, 0, 1, 0xBA, 0xC0],
        network: Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[0x33]),
        }),
    }
}

pub(super) fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, DEVICE_INSTANCE).unwrap()
}

pub(super) fn wildcard() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap()
}

pub(super) fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

pub(super) fn subscribe_cov(
    process: u32,
    object: ObjectIdentifier,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> (ConfirmedServiceChoice, BytesMut) {
    let mut request = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: confirmed,
        lifetime,
    }
    .encode(&mut request)
    .unwrap();
    (ConfirmedServiceChoice::SUBSCRIBE_COV, request)
}

/// `(process, property, index, increment, confirmed, lifetime)` on `object`.
pub(super) type PropertySubscription = (
    u32,
    PropertyIdentifier,
    Option<u32>,
    Option<f32>,
    Option<bool>,
    Option<u32>,
);

pub(super) fn subscribe_cov_property(
    object: ObjectIdentifier,
    (process, property, index, increment, confirmed, lifetime): PropertySubscription,
) -> (ConfirmedServiceChoice, BytesMut) {
    let mut request = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: confirmed,
        lifetime,
        monitored_property_identifier: property,
        monitored_property_array_index: index,
        cov_increment: increment,
    }
    .encode(&mut request)
    .unwrap();
    (ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, request)
}

/// ReadPropertyMultiple specifications: object plus `(property, index)` rows.
pub(super) type RpmSpecs = Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, Option<u32>)>)>;

fn rpm_request(specs: RpmSpecs) -> (ConfirmedServiceChoice, BytesMut) {
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: specs
            .into_iter()
            .map(|(object_identifier, references)| ReadAccessSpecification {
                object_identifier,
                list_of_property_references: references
                    .into_iter()
                    .map(
                        |(property_identifier, property_array_index)| PropertyReference {
                            property_identifier,
                            property_array_index,
                        },
                    )
                    .collect(),
            })
            .collect(),
    }
    .encode(&mut request)
    .unwrap();
    (ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, request)
}

/// One confirmed exchange through the MS/TP-style reply path.
pub(super) async fn exchange(
    tx: &mpsc::Sender<ReceivedNpdu>,
    peer: &Peer,
    invoke_id: u8,
    (service_choice, service_request): (ConfirmedServiceChoice, BytesMut),
) -> Apdu {
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            source: peer.network.clone(),
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(ReceivedNpdu {
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&peer.mac),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: Some(reply_tx),
    })
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), reply_rx)
        .await
        .expect("server response without lock inversion")
        .unwrap();
    let reply = decode_npdu(reply).unwrap();
    assert_eq!(reply.destination, peer.network, "reply routed to requester");
    decode_apdu(reply.payload).unwrap()
}

pub(super) struct Wire {
    pub(super) server: BACnetServer<WireTransport>,
    pub(super) tx: mpsc::Sender<ReceivedNpdu>,
    invoke_id: u8,
}

impl Wire {
    pub(super) async fn start(config: ServerConfig) -> Self {
        let (tx, rx) = mpsc::channel(16);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: DEVICE_INSTANCE,
                name: "Active COV Device".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let mut first = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        first.set_present_value(20.0);
        db.add(Box::new(first)).unwrap();
        db.add(Box::new(AnalogValueObject::new(2, "AV-2", 62).unwrap()))
            .unwrap();
        let server = BACnetServer::start(config, db, WireTransport { incoming: Some(rx) })
            .await
            .unwrap();
        Self {
            server,
            tx,
            invoke_id: 0,
        }
    }

    /// Exact duplicates are discarded by the tracker; keep every request distinct.
    pub(super) async fn send(
        &mut self,
        peer: &Peer,
        request: (ConfirmedServiceChoice, BytesMut),
    ) -> Apdu {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        exchange(&self.tx, peer, self.invoke_id, request).await
    }

    pub(super) async fn read(
        &mut self,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<Vec<u8>, (ErrorClass, ErrorCode)> {
        let mut request = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: index,
        }
        .encode(&mut request);
        let request = (ConfirmedServiceChoice::READ_PROPERTY, request);
        match self.send(&direct(), request).await {
            Apdu::ComplexAck(ack) => {
                let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
                assert_eq!(ack.property_identifier, property);
                Ok(ack.property_value)
            }
            Apdu::Error(error) => Err((error.error_class, error.error_code)),
            other => panic!("unexpected ReadProperty response {other:?}"),
        }
    }

    /// Encoded wire ReadProperty value of the selected Device's list.
    pub(super) async fn active_bytes(&mut self) -> Vec<u8> {
        self.read(device(), ACTIVE, None).await.unwrap()
    }

    pub(super) async fn active(&mut self) -> Vec<BACnetCOVSubscription> {
        decode_subscriptions(&self.active_bytes().await)
    }

    pub(super) async fn rpm(&mut self, specs: RpmSpecs) -> ReadPropertyMultipleACK {
        rpm_ack(self.send(&direct(), rpm_request(specs)).await)
    }
}

pub(super) fn simple_ack(response: Apdu) {
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
}

pub(super) fn error(response: Apdu, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(&response, Apdu::Error(e) if e.error_class == class && e.error_code == code),
        "expected {class:?}/{code:?}, got {response:?}"
    );
}

fn rpm_ack(response: Apdu) -> ReadPropertyMultipleACK {
    let Apdu::ComplexAck(ack) = response else {
        panic!("unexpected ReadPropertyMultiple response {response:?}");
    };
    ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap()
}

/// Every successful Active_COV_Subscriptions row in one RPM response.
pub(super) fn active_rows(ack: &ReadPropertyMultipleACK) -> Vec<Vec<u8>> {
    ack.list_of_read_access_results
        .iter()
        .flat_map(|result| &result.list_of_results)
        .filter(|row| row.property_identifier == ACTIVE)
        .map(|row| row.property_value.clone().expect("Active_COV row value"))
        .collect()
}

/// Independent test decoder for a `BACnetLIST of BACnetCOVSubscription`
/// (Clause 21): `[0] BACnetRecipientProcess`, `[1] BACnetObjectPropertyReference`,
/// `[2] BOOLEAN`, `[3] Unsigned`, `[4] REAL OPTIONAL`, concatenated bare.
pub(super) fn decode_subscriptions(data: &[u8]) -> Vec<BACnetCOVSubscription> {
    fn primitive(data: &[u8], pos: usize, number: u8) -> (&[u8], usize) {
        let (tag, start) = tags::decode_tag(data, pos).unwrap();
        assert!(tag.is_context(number), "expected [{number}] at {pos}");
        let end = start + tag.length as usize;
        (&data[start..end], end)
    }
    fn constructed(data: &[u8], pos: usize, number: u8) -> (&[u8], usize) {
        let (tag, start) = tags::decode_tag(data, pos).unwrap();
        assert!(tag.is_opening_tag(number), "expected [{number}] at {pos}");
        tags::extract_context_value(data, start, number).unwrap()
    }

    let mut subscriptions = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (process, next) = constructed(data, pos, 0);
        let (recipient_bytes, after_recipient) = constructed(process, 0, 0);
        let (recipient, used) = decode_recipient(recipient_bytes, 0).unwrap();
        assert_eq!(used, recipient_bytes.len());
        let (process_id, process_end) = primitive(process, after_recipient, 1);
        assert_eq!(process_end, process.len());
        let (reference, next) = constructed(data, next, 1);
        let (confirmed, next) = primitive(data, next, 2);
        assert_eq!(confirmed.len(), 1);
        let (time_remaining, mut next) = primitive(data, next, 3);
        let mut cov_increment = None;
        if next < data.len() && tags::decode_tag(data, next).unwrap().0.is_context(4) {
            let (increment, after) = primitive(data, next, 4);
            cov_increment = Some(primitives::decode_real(increment).unwrap());
            next = after;
        }
        subscriptions.push(BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient,
                process_identifier: primitives::decode_unsigned(process_id).unwrap() as u32,
            },
            monitored_property_reference: decode_object_property_reference(reference).unwrap(),
            issue_confirmed_notifications: confirmed[0] != 0,
            time_remaining: primitives::decode_unsigned(time_remaining).unwrap() as u32,
            cov_increment,
        });
        pos = next;
    }
    subscriptions
}

pub(super) fn address(peer: &Peer) -> BACnetRecipient {
    BACnetRecipient::Address(match &peer.network {
        Some(source) => BACnetAddress {
            network_number: source.network,
            mac_address: source.mac_address.clone(),
        },
        None => BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&peer.mac),
        },
    })
}

pub(super) fn processes(subscriptions: &[BACnetCOVSubscription]) -> Vec<u32> {
    let mut ids: Vec<_> = subscriptions
        .iter()
        .map(|entry| entry.recipient.process_identifier)
        .collect();
    ids.sort_unstable();
    ids
}

pub(super) fn find(
    subscriptions: &[BACnetCOVSubscription],
    process: u32,
) -> &BACnetCOVSubscription {
    let mut matching = subscriptions
        .iter()
        .filter(|entry| entry.recipient.process_identifier == process);
    let entry = matching
        .next()
        .unwrap_or_else(|| panic!("process {process} missing: {subscriptions:?}"));
    assert!(matching.next().is_none(), "process {process} duplicated");
    entry
}
