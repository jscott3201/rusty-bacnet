//! A request that arrives from a Device bound through this device's own
//! network number is tied to that Device, as sends to it take the binding as
//! a local one (#1358, #1404). The node is on this network, so its request
//! comes straight from its MAC with no SNET, or through a router here that
//! adds this network's number and the node's MAC as SNET and SADR. Either
//! way the Audit record names the Device as its source and the command it
//! writes publishes the Device as its Value_Source. While the number is
//! unknown, for a binding routed to another network, or when two bindings
//! name the same MAC, both name the address the request came from.
//!
//! Device 30 writes from `SOURCE`; requests reach the server as NPDUs on its
//! link and its answers are read back from there. Device 20 is the Audit
//! recipient, bound at `LOGGER`.
use super::local_network::{REMOTE_NETWORK, THIS_NETWORK, publish};
use super::*;
use bacnet_encoding::constructed::decode_value_source;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetValueSource};

/// The Device that writes from `SOURCE`.
const WRITER: u32 = 30;
/// The router a routed binding to the writer names. Nothing goes there: the
/// writer's requests arrive directly and are answered the same way.
const WRITER_ROUTER: &[u8] = &[5];
/// The writer's MAC when `SOURCE` is a router on this network relaying its
/// requests.
const RELAYED_WRITER: &[u8] = &[6];

/// A server reporting to Device 20 that binds the writer as `writer` lists,
/// and the sender that delivers NPDUs to its link.
async fn with_writer(writer: Vec<DeviceBinding>) -> (Fixture, mpsc::Sender<ReceivedNpdu>) {
    let (tx, rx) = mpsc::channel(4);
    let transport = AuditCapture::default();
    *transport.incoming.lock().unwrap() = Some(rx);
    let mut bindings = vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()];
    bindings.extend(writer);
    let fixture = try_servers_config(
        vec![reporter()],
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        bindings,
        true,
        1476,
        transport,
    )
    .await
    .unwrap();
    (fixture, tx)
}

/// The writer bound at `mac` on `network`, behind `WRITER_ROUTER`.
fn writer_routed(network: u16, mac: &[u8]) -> DeviceBinding {
    DeviceBinding::routed(oid(ObjectType::DEVICE, WRITER), network, mac, WRITER_ROUTER).unwrap()
}

/// Deliver a confirmed request from `SOURCE`, carrying `from` as its SNET and
/// SADR when one is given, and decode the answer sent back to `SOURCE`.
async fn request(
    fixture: &Fixture,
    tx: &mpsc::Sender<ReceivedNpdu>,
    from: Option<&NpduAddress>,
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    service_request: Bytes,
) -> Apdu {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
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
            service_request,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            source: from.cloned(),
            payload: apdu.freeze(),
            ..Default::default()
        },
    )
    .unwrap();
    let responses = &fixture.transport.responses;
    let answered = responses.lock().unwrap().len();
    tx.send(ReceivedNpdu::unverified(
        npdu.freeze(),
        MacAddr::from_slice(SOURCE),
        false,
        vec![],
        None,
    ))
    .await
    .unwrap();
    // The paused clock moves only once every task is idle.
    for _ in 0..100 {
        if responses.lock().unwrap().len() > answered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let answer = responses.lock().unwrap()[answered].clone();
    let npdu = decode_npdu(answer).unwrap();
    assert_eq!(npdu.destination.as_ref(), from, "answered the way it came");
    decode_apdu(npdu.payload).unwrap()
}

/// Write Binary Value 1 from `SOURCE`, relayed from `from` when one is given,
/// then read its Value_Source back the same way. Returns whom the Audit
/// record names as the writer and what the command published as its source.
async fn write_and_trace(
    fixture: &Fixture,
    tx: &mpsc::Sender<ReceivedNpdu>,
    from: Option<NpduAddress>,
) -> (BACnetRecipient, BACnetValueSource) {
    let value = oid(ObjectType::BINARY_VALUE, 1);
    let written = request(
        fixture,
        tx,
        from.as_ref(),
        1,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            value,
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x91, 1],
            None,
        ),
    )
    .await;
    assert!(matches!(written, Apdu::SimpleAck(_)), "{written:?}");
    let mut read = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: value,
        property_identifier: PropertyIdentifier::VALUE_SOURCE,
        property_array_index: None,
    }
    .encode(&mut read);
    let Apdu::ComplexAck(ack) = request(
        fixture,
        tx,
        from.as_ref(),
        2,
        ConfirmedServiceChoice::READ_PROPERTY,
        read.freeze(),
    )
    .await
    else {
        panic!("Value_Source read failed");
    };
    let encoded = ReadPropertyACK::decode(&ack.service_ack)
        .unwrap()
        .property_value;
    let (source, end) = decode_value_source(&encoded, 0).unwrap();
    assert_eq!(end, encoded.len());
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1, "one record, for the write");
    (records[0].notifications[0].source_device.clone(), source)
}

/// The writer's Device, as the Audit record and Value_Source name it.
fn writer_device() -> (BACnetRecipient, BACnetValueSource) {
    let device = oid(ObjectType::DEVICE, WRITER);
    (
        BACnetRecipient::Device(device),
        BACnetValueSource::Object(BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: device,
        }),
    )
}

/// The address the write came from, as both name it.
fn source_address() -> (BACnetRecipient, BACnetValueSource) {
    let address = BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(SOURCE),
    };
    (
        BACnetRecipient::Address(address.clone()),
        BACnetValueSource::Address(address),
    )
}

#[tokio::test(start_paused = true)]
async fn a_direct_write_from_a_binding_routed_through_this_network_names_its_device() {
    let (mut fixture, tx) = with_writer(vec![writer_routed(THIS_NETWORK, SOURCE)]).await;
    publish(&fixture, THIS_NETWORK);
    assert_eq!(write_and_trace(&fixture, &tx, None).await, writer_device());
    fixture.server.stop().await.unwrap();
}

/// `SOURCE` is a router on this network that passes the writer's requests
/// back onto it, adding this network's number and the writer's MAC as SNET
/// and SADR (Clause 6.5.4). That names the same node as a direct request, so
/// a binding through this network names the writer's Device, and so does a
/// local binding once the number is known.
#[tokio::test(start_paused = true)]
async fn a_write_relayed_with_this_networks_snet_names_the_device_bound_here() {
    for binding in [
        writer_routed(THIS_NETWORK, RELAYED_WRITER),
        DeviceBinding::local(oid(ObjectType::DEVICE, WRITER), RELAYED_WRITER).unwrap(),
    ] {
        let (mut fixture, tx) = with_writer(vec![binding.clone()]).await;
        publish(&fixture, THIS_NETWORK);
        let from = NpduAddress {
            network: THIS_NETWORK,
            mac_address: MacAddr::from_slice(RELAYED_WRITER),
        };
        assert_eq!(
            write_and_trace(&fixture, &tx, Some(from)).await,
            writer_device(),
            "{binding:?}"
        );
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_direct_write_names_its_address_while_the_number_is_unknown_or_bound_elsewhere() {
    for (network, published) in [(THIS_NETWORK, None), (REMOTE_NETWORK, Some(THIS_NETWORK))] {
        let (mut fixture, tx) = with_writer(vec![writer_routed(network, SOURCE)]).await;
        if let Some(number) = published {
            publish(&fixture, number);
        }
        assert_eq!(
            fixture.server.test_network().local_network_number().get(),
            published,
            "the number the case names"
        );
        assert_eq!(
            write_and_trace(&fixture, &tx, None).await,
            source_address(),
            "bound on {network}, number {published:?}"
        );
        fixture.server.stop().await.unwrap();
    }
}

/// Device 31 bound locally at `SOURCE` and the writer bound through this
/// network at the same MAC both name a direct request from there.
#[tokio::test(start_paused = true)]
async fn a_direct_write_named_by_two_bindings_stays_ambiguous() {
    let (mut fixture, tx) = with_writer(vec![
        DeviceBinding::local(oid(ObjectType::DEVICE, 31), SOURCE).unwrap(),
        writer_routed(THIS_NETWORK, SOURCE),
    ])
    .await;
    publish(&fixture, THIS_NETWORK);
    assert_eq!(write_and_trace(&fixture, &tx, None).await, source_address());
    fixture.server.stop().await.unwrap();
}
