use super::*;
use bacnet_transport::sc::ScActiveHub;
use bacnet_transport::tap::{FrameTap, TapDirection, TapPeer};
use bacnet_types::data_link::DataLink;
use bytes::BytesMut;
use std::future::Future;
use std::task::Waker;

const DEVICE: &str = "192.0.2.3:47808";
const BBMD: &str = "192.0.2.1:47808";
const ROUTER: &str = "192.0.2.9:47808";

/// An I-Am from device `instance`: max APDU 1476, no segmentation,
/// vendor 260, as the stack's encoder writes it.
fn i_am(instance: u32) -> Vec<u8> {
    let mut request = BytesMut::new();
    bacnet_services::who_is::IAmRequest {
        object_identifier: ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, instance)
            .unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 260,
    }
    .encode(&mut request);
    [&[0x10, 0x00][..], &request].concat()
}

/// `apdu` in an NPDU, from `source` (SNET, SADR) when a router relayed it.
fn npdu(apdu: &[u8], source: Option<(u16, u8)>) -> Vec<u8> {
    match source {
        None => [&[0x01, 0x00][..], apdu].concat(),
        Some((network, station)) => {
            let [high, low] = network.to_be_bytes();
            [&[0x01, 0x08, high, low, 0x01, station][..], apdu].concat()
        }
    }
}

fn bvll(function: u8, payload: &[u8]) -> Vec<u8> {
    let [high, low] = u16::try_from(payload.len() + 4).unwrap().to_be_bytes();
    [&[0x81, function, high, low][..], payload].concat()
}

/// A B/IP Original-Broadcast-NPDU, or a Forwarded-NPDU from `origin`.
fn bip(npdu: &[u8], origin: Option<&str>) -> Vec<u8> {
    match origin {
        None => bvll(0x0B, npdu),
        Some(origin) => {
            let SocketAddr::V4(origin) = origin.parse().unwrap() else {
                unreachable!()
            };
            let mac = bacnet_transport::bvll::encode_bip_mac(origin.ip().octets(), origin.port());
            bvll(0x04, &[&mac[..], npdu].concat())
        }
    }
}

/// The Python record `octets` make, received on `link` from `peer`.
fn record(link: DataLink, peer: TapPeer, octets: &[u8]) -> PyIAmArrival {
    let tap = FrameTap::new();
    let mut receiver = tap.subscribe();
    tap.record_for_test(TapDirection::Inbound, link, peer, octets);
    let frame = receiver.try_recv().unwrap();
    let arrival = IAmArrival::from_frame(&frame).unwrap().expect("an I-Am");
    PyIAmArrival::new(arrival, 0, Clock::fixed())
}

fn udp(text: &str) -> TapPeer {
    TapPeer::Udp(text.parse().unwrap())
}

/// Each link writes `link_sender` as `Frame.peer` does and `link_source`
/// and `device_address` in the address forms the calls take.
#[test]
fn each_link_writes_the_forms_frames_and_the_calls_use() {
    let direct = record(
        DataLink::Bip,
        udp(DEVICE),
        &bip(&npdu(&i_am(7), None), None),
    );
    assert_eq!(
        (direct.link_sender(), direct.link_source()),
        (DEVICE, DEVICE)
    );
    assert_eq!(direct.device_address(), Some(DEVICE));
    assert_eq!(
        (direct.forwarding_bbmd(), direct.next_hop_router()),
        (None, None)
    );
    assert_eq!(direct.link(), "bip");

    let forwarded = record(
        DataLink::Bip,
        udp(BBMD),
        &bip(&npdu(&i_am(7), None), Some(DEVICE)),
    );
    assert_eq!(forwarded.link_sender(), BBMD);
    assert_eq!(forwarded.link_source(), DEVICE);
    assert_eq!(forwarded.original_source(), Some(DEVICE));
    assert_eq!(forwarded.forwarding_bbmd(), Some(BBMD));
    assert_eq!(forwarded.device_address(), Some(DEVICE));

    let routed = record(
        DataLink::Bip,
        udp(ROUTER),
        &bip(&npdu(&i_am(7), Some((5, 7))), None),
    );
    assert_eq!(routed.device_address(), Some("5:0x07"));
    assert_eq!(routed.next_hop_router(), Some(ROUTER));
    assert_eq!(
        (routed.source_network(), routed.source_address()),
        (Some(5), Some(&[7][..]))
    );

    let mut wire = BytesMut::new();
    let local = npdu(&i_am(7), None);
    bacnet_transport::bip6::encode_bvll6(
        &mut wire,
        &bacnet_transport::bip6::Bvll6::OriginalBroadcastNpdu {
            source: bacnet_transport::bip6::Bip6Vmac::new([0, 1, 2]),
            npdu: &local,
        },
    )
    .unwrap();
    let v6 = record(DataLink::Bip6, udp("[2001:db8::2]:47808"), &wire);
    assert_eq!(v6.link_sender(), "[2001:db8::2]:47808");
    assert_eq!(v6.link_source(), "bip6:0x000102");
    assert_eq!(v6.link(), "ipv6");

    let message = bacnet_transport::sc_frame::ScMessage {
        function: bacnet_transport::sc_frame::ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([2, 0, 0, 0, 0, 1]),
        destination_vmac: Some([0xFF; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: local.clone().into(),
    };
    let mut wire = BytesMut::new();
    bacnet_transport::sc_frame::encode_sc_message(&mut wire, &message);
    let sc = record(DataLink::Sc, TapPeer::ScHub(ScActiveHub::Primary), &wire);
    assert_eq!(sc.link_sender(), "primary");
    assert_eq!(sc.link_source(), "02:00:00:00:00:01");
    assert_eq!(sc.device_address(), Some("02:00:00:00:00:01"));

    use bacnet_transport::mstp::codec::{Frame, FrameType};
    let mut wire = vec![0; 16 + local.len() * 2];
    let length = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 255, 9, &local)
        .encode_into(&mut wire)
        .unwrap();
    wire.truncate(length);
    let mstp = record(DataLink::Mstp, TapPeer::Mstp(9), &wire);
    assert_eq!(mstp.link_sender(), "9");
    assert_eq!(mstp.link_source(), "mstp:9");
    assert_eq!(mstp.device_address(), Some("mstp:9"));
}

/// Records compare, hash and copy by their fields (the Python suite
/// pickles them, with the module importable).
#[test]
fn records_compare_hash_and_rebuild_by_their_fields() {
    // DNET 9, station 19; SNET 5, station 7.
    let header = [1, 0x28, 0, 9, 1, 19, 0, 5, 1, 7, 255];
    let wire = bip(&[&header[..], &i_am(7)].concat(), Some(ROUTER));
    let one = record(DataLink::Bip, udp(BBMD), &wire);
    Python::initialize();
    Python::attach(|py| {
        let one = Py::new(py, one).unwrap().into_bound(py);
        let copy = py.import("copy").unwrap();
        for how in ["copy", "deepcopy"] {
            let copied = copy.call_method1(how, (&one,)).unwrap();
            assert!(copied.eq(&one).unwrap());
            assert_eq!(copied.hash().unwrap(), one.hash().unwrap());
            assert_eq!(
                copied.repr().unwrap().to_string(),
                "IAmArrival(instance=7, device_address='5:0x07', link_sender='192.0.2.1:47808')"
            );
        }
        let reduced = one.call_method0("__reduce__").unwrap();
        let rebuild = reduced.get_item(0).unwrap();
        let fields = reduced.get_item(1).unwrap().cast_into::<PyTuple>().unwrap();
        assert_eq!(fields.len(), 18);
        let restored = rebuild.call1(&fields).unwrap();
        assert!(restored.eq(&one).unwrap());
        assert_eq!(
            restored
                .getattr("destination_network")
                .unwrap()
                .extract::<u16>()
                .unwrap(),
            9
        );
        assert_eq!(
            restored
                .getattr("destination_address")
                .unwrap()
                .extract::<Vec<u8>>()
                .unwrap(),
            [19]
        );
        let previous = rebuild.call1(fields.get_slice(0, 16)).unwrap();
        assert!(previous.getattr("destination_network").unwrap().is_none());
        assert!(previous.getattr("destination_address").unwrap().is_none());
        assert_eq!(
            previous
                .getattr("source_address")
                .unwrap()
                .extract::<Vec<u8>>()
                .unwrap(),
            [7]
        );
        let other = record(DataLink::Bip, udp(ROUTER), &wire);
        assert!(!one.eq(Py::new(py, other).unwrap()).unwrap());
    });
}

/// Take the next fresh record as `__anext__` does, with its delivery hook.
async fn fresh(
    iterator: &PyIAmArrivalIterator,
) -> (PyIAmArrival, Arc<dyn crate::py_async::PutBack>) {
    let (next, hook) = iterator.subscription.stream_next();
    let next = tokio::time::timeout(std::time::Duration::from_secs(1), next)
        .await
        .expect("a record is queued");
    let Some(crate::event_stream::Next::Fresh(arrival)) = next else {
        panic!("expected a fresh record");
    };
    let gaps = iterator.subscription.gaps();
    (iterator.converter()(arrival, &gaps), hook)
}

fn inbound(tap: &FrameTap, octets: &[u8]) {
    tap.record_for_test(TapDirection::Inbound, DataLink::Bip, udp(DEVICE), octets);
}

/// A record lost on its way to Python counts in `missed`, and the next one
/// reports it with the gap it carried, so the gaps add up to the total.
#[tokio::test(start_paused = true)]
async fn a_record_lost_on_its_way_carries_into_the_next_gap() {
    let tap = FrameTap::new();
    let owner = FrameSubscriptions::default();
    let iterator = subscribe(&owner, tap.subscribe_with_capacity(1), None, Clock::fixed());
    let frame = bip(&npdu(&i_am(1), None), None);
    for _ in 0..3 {
        inbound(&tap, &frame);
    }
    let (first, hook) = fresh(&iterator).await;
    assert_eq!(first.missed_before, 0);
    hook.settled();
    assert_eq!(iterator.subscription.missed(), 2);
    inbound(&tap, &frame);
    let (lost, hook) = fresh(&iterator).await;
    assert_eq!(lost.missed_before, 2);
    drop((lost, hook));
    assert_eq!(
        iterator.subscription.missed(),
        3,
        "two overflowed, one lost on its way"
    );
    inbound(&tap, &frame);
    let (next, hook) = fresh(&iterator).await;
    assert_eq!(next.missed_before, 3);
    hook.settled();
    assert_eq!(iterator.subscription.missed(), 3);
}

/// The owner's close ends its arrival iterators as it ends its frame
/// iterators: the tap subscription goes and a pending read ends; the
/// malformed and own-echo counts stay readable.
#[tokio::test(start_paused = true)]
async fn the_owners_close_ends_arrival_iterators_too() {
    let tap = FrameTap::new();
    let owner = FrameSubscriptions::default();
    let own = Some((DEVICE.parse().unwrap(), 1));
    let iterator = subscribe(&owner, tap.subscribe(), own, Clock::fixed());
    let frames = owner.subscribe(tap.subscribe(), "local".into(), Clock::fixed());
    inbound(&tap, &bip(&npdu(&[0x10, 0x00, 0xC4], None), None));
    // This owner's own I-Am come back to it.
    inbound(&tap, &bip(&npdu(&i_am(1), None), None));
    let (pending, _hook) = iterator.subscription.stream_next();
    tokio::pin!(pending);
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!((iterator.malformed(), iterator.own_echoes()), (1, 1));
    assert_eq!(tap.counters().subscribers, 2);
    owner.close();
    assert_eq!(tap.counters().subscribers, 0);
    assert!(pending.await.is_none());
    assert_eq!((iterator.malformed(), iterator.own_echoes()), (1, 1));
    drop(frames);
}
