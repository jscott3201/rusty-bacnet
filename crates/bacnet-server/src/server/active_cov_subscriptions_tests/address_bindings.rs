//! The Device's Device_Address_Binding lists the server's device bindings
//! (#1369, Clause 12.11.34): configured ones and I-Am observations under ten
//! minutes old, read through the same request paths and sampling as the
//! Device's COV lists.
use super::*;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};

const BINDINGS: PropertyIdentifier = PropertyIdentifier::DEVICE_ADDRESS_BINDING;
const ROUTER: [u8; 6] = [0x0A, 0, 0, 1, 0xBA, 0xC0];

fn peer(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// `(device, network, MAC)` of each BACnetAddressBinding in `data`, decoded
/// independently: an application-tagged identifier, network number and MAC.
fn decode(data: &[u8]) -> Vec<(ObjectIdentifier, u64, Vec<u8>)> {
    let next = |pos: usize| {
        bacnet_encoding::primitives::decode_application_value(data, pos)
            .expect("an application-tagged value")
    };
    let mut bindings = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (PropertyValue::ObjectIdentifier(device), at) = next(pos) else {
            panic!("a binding starts with its Device identifier");
        };
        let (PropertyValue::Unsigned(network), at) = next(at) else {
            panic!("then its network number");
        };
        let (PropertyValue::OctetString(mac), at) = next(at) else {
            panic!("then its MAC");
        };
        bindings.push((device, network, mac));
        pos = at;
    }
    bindings
}

/// Hear Device `instance`'s I-Am from `mac`, relayed from `source` when
/// routed, and wait until the binding table holds it.
async fn hear(wire: &Wire, instance: u32, mac: &[u8], source: Option<NpduAddress>) {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: peer(instance),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 260,
    }
    .encode(&mut service);
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source,
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    wire.tx
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(mac),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let resolution = wire.server.device_bindings.read().await.resolve_at(
                &peer(instance),
                runtime_clock::now(),
                |_| false,
            );
            if !matches!(
                resolution,
                super::super::device_bindings::DeviceResolution::Unknown
            ) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the I-Am binds the device");
}

#[tokio::test(start_paused = true)]
async fn an_i_am_heard_on_the_wire_is_listed_with_its_network() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    assert_eq!(wire.read(device(), BINDINGS, None).await, Ok(Vec::new()));

    // Device 9 answers from this network, Device 7 from network 77 through
    // a router: the binding names the device's own MAC there.
    let local_mac = [0x0A, 0, 0, 9, 0xBA, 0xC0];
    hear(&wire, 9, &local_mac, None).await;
    let routed = NpduAddress {
        network: 77,
        mac_address: MacAddr::from_slice(&[0x07]),
    };
    hear(&wire, 7, &ROUTER, Some(routed)).await;

    let read = wire.read(device(), BINDINGS, None).await.unwrap();
    assert_eq!(
        decode(&read),
        vec![(peer(7), 77, vec![0x07]), (peer(9), 0, local_mac.to_vec())]
    );
    // The local read serves the same list, one item per binding.
    let PropertyValue::List(items) = wire
        .server
        .read_local(&device(), BINDINGS, None)
        .await
        .unwrap()
    else {
        panic!("Device_Address_Binding is a list");
    };
    let joined: Vec<u8> = items
        .iter()
        .flat_map(|item| match item {
            PropertyValue::ApplicationData(bytes) => bytes.clone(),
            other => panic!("an encoded binding, got {other:?}"),
        })
        .collect();
    assert_eq!(items.len(), 2);
    assert_eq!(joined, read);
    // It is a list, not an array, and only the executor writes it.
    assert_eq!(
        wire.read(device(), BINDINGS, Some(1)).await,
        Err((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
    );
    wire.server.stop().await.unwrap();
}

/// The Device_Address_Binding row of a ReadPropertyMultiple-ACK's `results`.
fn bindings_row(results: Vec<bacnet_services::rpm::ReadAccessResult>) -> Vec<u8> {
    results
        .into_iter()
        .flat_map(|result| result.list_of_results)
        .find(|row| row.property_identifier == BINDINGS)
        .and_then(|row| row.property_value)
        .expect("a Device_Address_Binding row")
}

/// An observation is listed until its binding lapses, exactly
/// `OBSERVED_BINDING_TTL` after its I-Am: the table measures that on tokio's
/// clock (#1556), so the paused clock steps the edge. 1 ms before it the
/// device is listed, and at it the stale binding is not.
#[tokio::test(start_paused = true)]
async fn an_observation_is_listed_until_its_binding_lapses() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let local_mac = [0x0A, 0, 0, 9, 0xBA, 0xC0];
    // The server takes the I-Am before the clock can move.
    let heard = tokio::time::Instant::now();
    hear(&wire, 9, &local_mac, None).await;
    let lapses = heard + super::super::device_bindings::OBSERVED_BINDING_TTL;

    tokio::time::advance(lapses - Duration::from_millis(1) - tokio::time::Instant::now()).await;
    let read = wire.read(device(), BINDINGS, None).await.unwrap();
    assert_eq!(decode(&read), vec![(peer(9), 0, local_mac.to_vec())]);

    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(wire.read(device(), BINDINGS, None).await, Ok(Vec::new()));
    wire.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn configured_bindings_are_listed_through_every_read_path() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let local_mac = [0x0A, 0, 0, 21, 0xBA, 0xC0];
    {
        let mut table = wire.server.device_bindings.write().await;
        table
            .insert_configured(DeviceBinding::local(peer(21), local_mac).unwrap(), |_| {
                false
            })
            .unwrap();
        table
            .insert_configured(
                DeviceBinding::routed(peer(20), 5, [0x14], ROUTER).unwrap(),
                |_| false,
            )
            .unwrap();
    }
    let configured = vec![(peer(20), 5, vec![0x14]), (peer(21), 0, local_mac.to_vec())];
    let read = wire.read(device(), BINDINGS, None).await.unwrap();
    assert_eq!(decode(&read), configured);

    // ReadPropertyMultiple's ALL and REQUIRED serve the same value as
    // ReadProperty: the property is required of every Device.
    for selector in [PropertyIdentifier::ALL, PropertyIdentifier::REQUIRED] {
        let ack = wire.rpm(vec![(device(), vec![(selector, None)])]).await;
        assert_eq!(
            bindings_row(ack.list_of_read_access_results),
            read,
            "{selector:?}"
        );
    }

    // So does a Group member naming it, through the Group's Present_Value.
    super::group_present_value::add_group(&wire, 1, &[(device(), &[BINDINGS])]).await;
    let group = super::group_present_value::group(1);
    let present_value = wire
        .read(group, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
    let members = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&present_value).unwrap();
    assert_eq!(bindings_row(members.list_of_read_access_results), read);

    // ReadRange pages it one binding per item.
    let mut request = BytesMut::new();
    ReadRangeRequest {
        object_identifier: device(),
        property_identifier: BINDINGS,
        property_array_index: None,
        range: Some(RangeSpec::ByPosition {
            reference_index: 2,
            count: 1,
        }),
    }
    .encode(&mut request)
    .unwrap();
    let Apdu::ComplexAck(ack) = wire
        .send(&direct(), (ConfirmedServiceChoice::READ_RANGE, request))
        .await
    else {
        panic!("a ReadRange-ACK");
    };
    let ack = ReadRangeAck::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.item_count, 1);
    assert_eq!(decode(&ack.item_data), configured[1..].to_vec());

    // The bindings outlast `stop()`, and the local read still lists them.
    wire.server.stop().await.unwrap();
    let PropertyValue::List(items) = wire
        .server
        .read_local(&device(), BINDINGS, None)
        .await
        .unwrap()
    else {
        panic!("Device_Address_Binding is a list");
    };
    assert_eq!(items.len(), 2);
}
