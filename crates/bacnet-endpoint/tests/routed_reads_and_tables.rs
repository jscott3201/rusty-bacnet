//! A routed paged log read and a BBMD table read through one B/IP endpoint
//! (#2612), against a raw UDP peer on real loopback: the peer plays the
//! router, so every request's NPDU destination can be read off the wire,
//! and the BBMD.
//!
//! Real time, as the other real-UDP endpoint tests run: paused time over
//! real UDP flakes.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use bacnet_client::log_reader::LogCursor;
use bacnet_encoding::apdu::{Apdu, ComplexAck, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_endpoint::EndpointApduDestination;
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_npdu::NetworkPriority;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_transport::bip::BvlcExchangeError;
use bacnet_transport::bvll::{Bvll, decode_bvll, encode_bvll};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use tokio::net::UdpSocket;

const WAIT: Duration = Duration::from_secs(5);
const NETWORK: u16 = 77;
const STATION: [u8; 1] = [0x44];

fn log() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_LOG, 1).unwrap()
}

/// A raw peer on loopback and its six-octet B/IP MAC.
async fn peer() -> (UdpSocket, MacAddr) {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut mac = MacAddr::from_slice(&Ipv4Addr::LOCALHOST.octets());
    mac.extend_from_slice(&socket.local_addr().unwrap().port().to_be_bytes());
    (socket, mac)
}

/// The next datagram `peer` receives, and who sent it.
async fn receive(peer: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buf = vec![0; 2048];
    let (len, source) = tokio::time::timeout(WAIT, peer.recv_from(&mut buf))
        .await
        .expect("a request arrives")
        .unwrap();
    buf.truncate(len);
    (buf, source)
}

/// The NPDU's destination and its confirmed request.
fn routed_request(datagram: &[u8]) -> (Option<NpduAddress>, ConfirmedRequest) {
    let bvll = decode_bvll(datagram).unwrap();
    let npdu = decode_npdu(Bytes::copy_from_slice(&bvll.payload)).unwrap();
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::ConfirmedRequest(request) => (npdu.destination, request),
        other => panic!("expected a confirmed request, got {other:?}"),
    }
}

/// `service_ack` answering `request`, relayed from the station as a router
/// relays it: SNET/SADR naming it.
fn routed_ack(request: &ConfirmedRequest, service_ack: BytesMut) -> Vec<u8> {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: request.service_choice,
            service_ack: service_ack.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: false,
            priority: NetworkPriority::Normal,
            destination: None,
            source: Some(NpduAddress {
                network: NETWORK,
                mac_address: MacAddr::from_slice(&STATION),
            }),
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let mut frame = BytesMut::new();
    encode_bvll(&mut frame, &Bvll::OriginalUnicastNpdu(&npdu)).unwrap();
    frame.to_vec()
}

/// Answer one routed ReadProperty of a log count with `count`, after
/// checking it went to the station behind the router.
async fn answer_count(router: &UdpSocket, expected: PropertyIdentifier, count: u64) {
    let (datagram, source) = receive(router).await;
    let (destination, request) = routed_request(&datagram);
    let destination = destination.expect("a routed request carries DNET/DADR");
    assert_eq!(destination.network, NETWORK);
    assert_eq!(destination.mac_address.as_slice(), STATION);
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::READ_PROPERTY
    );
    let read = ReadPropertyRequest::decode(&request.service_request).unwrap();
    assert_eq!(
        (read.object_identifier, read.property_identifier),
        (log(), expected)
    );
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut value, &PropertyValue::Unsigned(count))
        .unwrap();
    let mut ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: log(),
        property_identifier: expected,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut ack)
    .unwrap();
    router
        .send_to(&routed_ack(&request, ack), source)
        .await
        .unwrap();
}

/// Every request of a routed log page goes to the station behind the
/// router, the count reads and the ReadRange alike, and the station's
/// relayed answers complete it (#2612).
#[tokio::test]
async fn a_routed_log_page_sends_every_request_to_the_station() {
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ClientOnly)
        .client_timers(3_000, 0)
        .build_session()
        .unwrap();
    endpoint.start().await.unwrap();
    let (router, router_mac) = peer().await;
    let destination = EndpointApduDestination::Routed {
        destination_network: NETWORK,
        destination_mac: MacAddr::from_slice(&STATION),
        router_mac,
    };

    // From the oldest record: three count reads find an empty log.
    let client = endpoint.cloned_client_handle().unwrap();
    let to = destination.clone();
    let page = tokio::spawn(async move {
        client
            .read_log_page_with_destination(to, Vec::new(), log(), LogCursor::Oldest, 10)
            .await
    });
    answer_count(&router, PropertyIdentifier::TOTAL_RECORD_COUNT, 0).await;
    answer_count(&router, PropertyIdentifier::RECORD_COUNT, 0).await;
    answer_count(&router, PropertyIdentifier::TOTAL_RECORD_COUNT, 0).await;
    let page = tokio::time::timeout(WAIT, page)
        .await
        .expect("the page completes")
        .unwrap()
        .unwrap();
    assert!(page.done);
    assert!(page.records.is_empty());
    assert_eq!(page.next, LogCursor::Sequence(1));

    // By position: one ReadRange, answered with no items.
    let client = endpoint.cloned_client_handle().unwrap();
    let page = tokio::spawn(async move {
        client
            .read_log_page_with_destination(
                destination,
                Vec::new(),
                log(),
                LogCursor::Position(1),
                10,
            )
            .await
    });
    let (datagram, source) = receive(&router).await;
    let (to, request) = routed_request(&datagram);
    let to = to.expect("a routed request carries DNET/DADR");
    assert_eq!(
        (to.network, to.mac_address.as_slice()),
        (NETWORK, &STATION[..])
    );
    assert_eq!(request.service_choice, ConfirmedServiceChoice::READ_RANGE);
    let read = ReadRangeRequest::decode(&request.service_request).unwrap();
    assert_eq!(read.object_identifier, log());
    assert!(matches!(
        read.range,
        Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 10
        })
    ));
    let mut ack = BytesMut::new();
    ReadRangeAck {
        object_identifier: log(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (false, false, false),
        item_count: 0,
        item_data: Vec::new(),
        first_sequence_number: None,
    }
    .encode(&mut ack);
    router
        .send_to(&routed_ack(&request, ack), source)
        .await
        .unwrap();
    let page = tokio::time::timeout(WAIT, page)
        .await
        .expect("the page completes")
        .unwrap()
        .unwrap();
    assert!(page.done);
    assert!(page.records.is_empty());
    assert_eq!(endpoint.active_leases(), 0);
    endpoint.stop().await.unwrap();
}

fn not_started(error: BvlcExchangeError) {
    match error {
        BvlcExchangeError::Transport(Error::Transport(io)) => {
            assert_eq!(io.kind(), std::io::ErrorKind::NotConnected);
        }
        other => panic!("not the not-started error: {other}"),
    }
}

/// The session's table reader sends through the session's own socket while
/// a read the client role started is in flight on it, fails while the
/// session is stopped, and only a B/IP builder's session has one (#2612).
#[tokio::test]
async fn the_session_reads_a_bbmd_table_through_its_own_socket() {
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ClientOnly)
        .client_timers(3_000, 0)
        .build_session()
        .unwrap();
    let tables = endpoint.bvlc_tables().expect("a B/IP session reads tables");
    let (bbmd, bbmd_mac) = peer().await;
    not_started(tables.read_bdt(&bbmd_mac).await.unwrap_err());
    endpoint.start().await.unwrap();
    let local = endpoint.bip_local_address().unwrap();

    // A ReadProperty to the same peer goes out first and stays unanswered.
    let client = endpoint.cloned_client_handle().unwrap();
    let to = bbmd_mac.clone();
    let read = tokio::spawn(async move {
        client
            .read_property(&to, log(), PropertyIdentifier::RECORD_COUNT, None)
            .await
    });
    let (datagram, source) = receive(&bbmd).await;
    assert_eq!(source.port(), local.port());
    let (_, request) = routed_request(&datagram);

    // Read-FDT while it is in flight: one entry, 127.0.0.9:47808, TTL 60,
    // 75 seconds left.
    let answer = async {
        let (datagram, source) = receive(&bbmd).await;
        assert_eq!(datagram, [0x81, 6, 0, 4]);
        assert_eq!(source.port(), local.port());
        let ack = [0x81, 7, 0, 14, 127, 0, 0, 9, 0xBA, 0xC0, 0, 60, 0, 75];
        bbmd.send_to(&ack, source).await.unwrap();
    };
    let (rows, ()) = tokio::join!(tables.read_fdt(&bbmd_mac), answer);
    let rows = rows.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].ip, rows[0].port, rows[0].ttl),
        ([127, 0, 0, 9], 47808, 60)
    );
    assert!(!read.is_finished());

    // The read still completes afterwards.
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut value, &PropertyValue::Unsigned(3))
        .unwrap();
    let mut ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: log(),
        property_identifier: PropertyIdentifier::RECORD_COUNT,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut ack)
    .unwrap();
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: ack.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let mut frame = BytesMut::new();
    encode_bvll(&mut frame, &Bvll::OriginalUnicastNpdu(&npdu)).unwrap();
    bbmd.send_to(&frame, source).await.unwrap();
    let ack = tokio::time::timeout(WAIT, read)
        .await
        .expect("the read completes")
        .unwrap()
        .unwrap();
    assert_eq!(ack.property_identifier, PropertyIdentifier::RECORD_COUNT);

    endpoint.stop().await.unwrap();
    not_started(tables.read_bdt(&bbmd_mac).await.unwrap_err());

    let other = bacnet_endpoint::session::EndpointSession::new(
        bacnet_transport::bip::BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
        SessionRole::ClientOnly,
        bacnet_endpoint::session::SessionConfig::default(),
    )
    .unwrap();
    assert!(other.bvlc_tables().is_none());
}

#[path = "support/eventually.rs"]
mod eventually;

/// A ComplexAck to ReadProperty of `log()`'s Record_Count with `count`,
/// relayed from the station behind a router: SNET/SADR naming it.
fn relayed_count(invoke_id: u8, count: u64) -> Bytes {
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut value, &PropertyValue::Unsigned(count))
        .unwrap();
    let mut ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: log(),
        property_identifier: PropertyIdentifier::RECORD_COUNT,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut ack)
    .unwrap();
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: ack.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source: Some(NpduAddress {
                network: NETWORK,
                mac_address: MacAddr::from_slice(&STATION),
            }),
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.freeze()
}

/// With no router known, a read goes as one broadcast on the local link
/// naming the station's network and MAC (Clause 6.5.3). When two routers
/// both forward it, the first answer completes the read, the second is
/// dropped and counted, and no lease is left; a network no router answers
/// for times out (#2612).
#[tokio::test(start_paused = true)]
async fn a_read_by_local_broadcast_takes_the_first_of_two_answers() {
    use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
    use bacnet_transport::test_transport::{BIP_LOCAL_MAC, TestTransport};

    let (inbound, receiver) = tokio::sync::mpsc::channel(8);
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .inbound(receiver)
        .build();
    let sent = transport.sent();
    let mut session = bacnet_endpoint::session::EndpointSession::new(
        transport,
        SessionRole::ClientOnly,
        bacnet_endpoint::session::SessionConfig {
            apdu_timeout_ms: 1_000,
            apdu_retries: 0,
            ..bacnet_endpoint::session::SessionConfig::default()
        },
    )
    .unwrap();
    session.start().await.unwrap();
    let by_broadcast = |network| EndpointApduDestination::RoutedViaLocalBroadcast {
        destination_network: network,
        destination_mac: MacAddr::from_slice(&STATION),
    };
    let client = session.cloned_client_handle().unwrap();
    let to = by_broadcast(NETWORK);
    let read = tokio::spawn(async move {
        client
            .read_property_with_destination(
                to,
                Vec::new(),
                log(),
                PropertyIdentifier::RECORD_COUNT,
                None,
            )
            .await
    });
    eventually::eventually(WAIT, "the broadcast goes out", async || {
        sent.broadcasts().len() == 1
    })
    .await;
    let frame = sent.broadcasts().remove(0);
    assert!(frame.broadcast);
    let npdu = frame.decode_npdu();
    let destination = npdu
        .destination
        .clone()
        .expect("DNET/DADR on the broadcast");
    assert_eq!(
        (destination.network, destination.mac_address.as_slice()),
        (NETWORK, &STATION[..])
    );
    assert_eq!(npdu.hop_count, 255);
    assert!(npdu.expecting_reply);
    let Apdu::ConfirmedRequest(request) = frame.apdu() else {
        panic!("a confirmed request")
    };
    assert_eq!(session.active_leases(), 1);
    // Both routers on the link forward it, and both relay the answer.
    for (router, count) in [
        ([10, 0, 0, 1, 0xBA, 0xC0], 3),
        ([10, 0, 0, 2, 0xBA, 0xC0], 4),
    ] {
        inbound
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: relayed_count(request.invoke_id, count),
                source_mac: MacAddr::from_slice(&router),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
    }
    let ack = read.await.unwrap().unwrap();
    assert_eq!(ack.property_value, [0x21, 3]);
    eventually::eventually(WAIT, "the second answer is counted", async || {
        session.policy_counters().await.unclaimed_terminal == 1
    })
    .await;
    assert_eq!(session.active_leases(), 0);

    // A network no router answers for: the read ends at its timeout.
    let client = session.cloned_client_handle().unwrap();
    let silent = client
        .read_property_with_destination(
            by_broadcast(9),
            Vec::new(),
            log(),
            PropertyIdentifier::RECORD_COUNT,
            None,
        )
        .await;
    assert!(matches!(silent, Err(Error::Timeout(_))), "{silent:?}");
    assert_eq!(sent.broadcasts().len(), 2);
    assert_eq!(session.active_leases(), 0);
    session.stop().await.unwrap();
}
