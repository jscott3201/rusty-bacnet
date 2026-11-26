//! The checks on a routed confirmed request, before anything is sent: a
//! router that is a broadcast or group address on this link (#2722), and a
//! remote MAC that is its network's broadcast (#2709); and the form that
//! reaches a network by one local broadcast while no router is known
//! (#2723).
//!
//! Each link's own group rule decides what a router may be: the client asks
//! the transport. What was sent is read from the shared test transport's
//! send log.
use std::net::Ipv4Addr;

use bacnet_encoding::apdu::{Apdu, ComplexAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::mstp::{LoopbackSerial, MstpTransport};
use bacnet_transport::port::{GroupDestinations, ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport};
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::data_link::DataLink;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use bytes::BytesMut;
use tokio::sync::mpsc;

use super::{BACnetClient, ClientOptions, Station};

const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;
const NETWORK: u16 = 5;
const STATION: &[u8] = &[7];

/// A client on a link named `link` whose group destinations follow
/// `groups`, with `options` and `retries` per request, the test feeding its
/// inbound frames.
async fn client_on(
    link: DataLink,
    local_mac: &[u8],
    groups: GroupDestinations,
    options: ClientOptions,
    retries: u8,
) -> (
    BACnetClient<TestTransport>,
    SendLog,
    mpsc::Sender<ReceivedNpdu>,
) {
    let (inbound_tx, inbound_rx) = mpsc::channel(8);
    let transport = TestTransport::builder()
        .local_mac(local_mac)
        .inbound(inbound_rx)
        .data_link(link)
        .on_is_group_destination(move |mac| groups.contains(mac))
        .build();
    let sent = transport.sent();
    let config = super::ClientConfig {
        apdu_timeout_ms: 20,
        apdu_retries: retries,
        ..super::ClientConfig::default()
    };
    let client = BACnetClient::start_with_options(config, transport, options)
        .await
        .unwrap();
    (client, sent, inbound_tx)
}

fn timed_out<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Abort { reason }))
        if *reason == AbortReason::TSM_TIMEOUT)
}

fn refused_with<T: std::fmt::Debug>(result: &Result<T, Error>, text: &str) -> bool {
    matches!(result, Err(Error::InvalidArgument(message)) if message.to_string().contains(text))
}

/// Each link's group routers, by its own rule, and one router that is a
/// station there.
struct LinkCase {
    link: DataLink,
    local_mac: Vec<u8>,
    groups: GroupDestinations,
    group_routers: Vec<(Vec<u8>, &'static str)>,
    router: Vec<u8>,
}

fn link_cases() -> Vec<LinkCase> {
    let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0xBAC0, Ipv4Addr::new(192, 168, 1, 255));
    let mstp = MstpTransport::new(LoopbackSerial::pair().0, Default::default());
    let sc = ScTransport::new(LoopbackWebSocket::pair().0, [0x02; 6]);
    vec![
        LinkCase {
            link: DataLink::Bip,
            local_mac: vec![192, 168, 1, 2, 0xBA, 0xC0],
            groups: bip.group_destinations(),
            group_routers: vec![
                (
                    vec![255, 255, 255, 255, 0xBA, 0xC0],
                    "255.255.255.255:47808",
                ),
                (vec![192, 168, 1, 255, 0xBA, 0xC1], "192.168.1.255:47809"),
                (vec![239, 255, 0, 1, 0xBA, 0xC0], "239.255.0.1:47808"),
            ],
            router: vec![192, 168, 1, 9, 0xBA, 0xC0],
        },
        LinkCase {
            link: DataLink::Mstp,
            local_mac: vec![1],
            groups: mstp.group_destinations(),
            group_routers: vec![(vec![0xFF], "MS/TP station 255")],
            router: vec![9],
        },
        LinkCase {
            link: DataLink::Sc,
            local_mac: vec![0x02; 6],
            groups: sc.group_destinations(),
            group_routers: vec![(vec![0xFF; 6], "FF:FF:FF:FF:FF:FF")],
            router: vec![0x02, 0, 0, 0, 0, 9],
        },
        // The IEEE 802 rule the Ethernet transport keeps (built on Linux
        // only): the low bit of the first octet marks a group.
        LinkCase {
            link: DataLink::Ethernet,
            local_mac: vec![0x02, 0, 0, 0, 0, 1],
            groups: GroupDestinations::new(|mac| mac.len() == 6 && mac[0] & 1 == 1),
            group_routers: vec![
                (vec![0xFF; 6], "FF:FF:FF:FF:FF:FF"),
                (vec![0x01, 0x80, 0xC2, 0, 0, 0], "01:80:C2:00:00:00"),
            ],
            router: vec![0x02, 0, 0, 0, 0, 9],
        },
    ]
}

/// On every link, a routed confirmed request through a router that is the
/// link's broadcast or a group address there is refused before any state
/// and before anything is sent, naming the router as the link writes it; a
/// router that is one station takes the same request (#2722).
#[tokio::test(start_paused = true)]
async fn a_routed_request_through_a_group_router_is_refused_on_every_link() {
    for case in link_cases() {
        let (mut client, sent, _inbound) = client_on(
            case.link,
            &case.local_mac,
            case.groups,
            ClientOptions::default(),
            0,
        )
        .await;
        for (router, text) in &case.group_routers {
            let to = Station::routed(router, NETWORK, STATION);
            let refused = client.confirmed_request(&to, READ, &[0x0C]).await;
            assert!(
                refused_with(
                    &refused,
                    &format!(
                        "the router {text} is a broadcast or group address on this {} link",
                        case.link
                    )
                ),
                "{}: {refused:?}",
                case.link
            );
            // The same through a table row naming that router.
            client
                .add_routed_device(crate::discovery::RoutedDeviceConfig {
                    instance: 9,
                    router_mac: router.clone(),
                    remote_network: NETWORK,
                    remote_mac: STATION.to_vec(),
                    max_apdu_length: 480,
                    segmentation_supported: bacnet_types::enums::Segmentation::NO_SEGMENTATION,
                    max_segments_accepted: None,
                })
                .await
                .unwrap();
            let refused = client
                .confirmed_request(&Station::remote(NETWORK, STATION), READ, &[0x0C])
                .await;
            assert!(refused_with(&refused, "broadcast or group"), "{refused:?}");
        }
        assert!(sent.is_empty(), "{}: a refused request was sent", case.link);
        assert_eq!(client.tsm.lock().pending_count(), 0);
        assert_eq!(client.tsm.lock().coordinated_active_count(), 0);

        let sent_through = client
            .confirmed_request(
                &Station::routed(&case.router, NETWORK, STATION),
                READ,
                &[0x0C],
            )
            .await;
        assert!(timed_out(&sent_through), "{}: {sent_through:?}", case.link);
        let [frame] = sent.frames().try_into().unwrap();
        assert!(!frame.broadcast);
        assert_eq!(frame.mac.as_slice(), case.router.as_slice());
        client.stop().await.unwrap();
    }
}

/// A B/IPv6 MAC is a 3-octet VMAC, which never names a group: any router's
/// VMAC takes a routed request, so there is nothing for the check to refuse.
#[cfg(feature = "ipv6")]
#[tokio::test(start_paused = true)]
async fn a_bip6_router_vmac_is_never_a_group() {
    use bacnet_transport::bip6::Bip6Transport;
    let bip6 = Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 0, Some(10));
    let (mut client, sent, _inbound) = client_on(
        DataLink::Bip6,
        &[0, 0, 10],
        bip6.group_destinations(),
        ClientOptions::default(),
        0,
    )
    .await;
    for router in [[0xFF; 3], [0x00, 0x04, 0xD2]] {
        let result = client
            .confirmed_request(&Station::routed(&router, NETWORK, STATION), READ, &[0x0C])
            .await;
        assert!(timed_out(&result), "{router:02X?}: {result:?}");
    }
    assert_eq!(sent.unicasts().len(), 2);
    client.stop().await.unwrap();
}

/// A link with no group rule: only the empty MAC, below, is special.
fn no_groups() -> GroupDestinations {
    GroupDestinations::new(|_| false)
}

/// A confirmed request to a station whose MAC on its own network is that
/// network's broadcast is refused in every station form before anything is
/// sent or looked up, and the discovery table gains nothing (#2709). An
/// unconfirmed request to one is left alone.
#[tokio::test(start_paused = true)]
async fn a_confirmed_request_to_a_remote_broadcast_form_is_refused_in_every_form() {
    let (mut client, sent, _inbound) = client_on(
        DataLink::Mstp,
        &[1],
        no_groups(),
        ClientOptions::default(),
        0,
    )
    .await;
    let router = [9];
    for (mac, form) in [
        (
            &[0xFF][..],
            "station 255, the broadcast of an MS/TP network",
        ),
        (&[0xFF; 3][..], "VMAC 0xFFFFFF"),
        (&[0x00, 0x05][..], "a LonTalk broadcast (subnet 5)"),
        (&[0xFF, 0x07][..], "LonTalk group 7, a multicast"),
        (
            &[0xFF; 6][..],
            "5:0xFFFFFFFFFFFF on network 5 is a broadcast on every six-octet link",
        ),
        (
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0][..],
            "255.255.255.255:47808, the limited broadcast",
        ),
    ] {
        let address = BACnetAddress {
            network_number: NETWORK,
            mac_address: MacAddr::from_slice(mac),
        };
        let forms = [
            Station::routed(&router, NETWORK, mac),
            Station::remote(NETWORK, mac),
            Station::routed_via_local_broadcast(NETWORK, mac),
            Station::from(address.clone()),
        ];
        for to in &forms {
            let refused = client.confirmed_request(to, READ, &[0x0C]).await;
            assert!(refused_with(&refused, form), "{to:?}: {refused:?}");
            assert!(
                refused_with(&refused, "only an unconfirmed request"),
                "{refused:?}"
            );
            assert!(to.check_confirmed().is_err(), "{to:?}");
        }
        let refused = client.confirmed_request(&address, READ, &[0x0C]).await;
        assert!(refused_with(&refused, form), "{refused:?}");
    }
    assert!(sent.is_empty(), "a refused request was sent");
    assert_eq!(client.tsm.lock().pending_count(), 0);
    assert!(client.discovered_devices().await.is_empty());

    // Unconfirmed: sent through the router as named.
    client
        .unconfirmed_request(
            &Station::routed(&router, NETWORK, &[0xFF]),
            UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
            &[],
        )
        .await
        .unwrap();
    let [frame] = sent.frames().try_into().unwrap();
    let npdu = frame.decode_npdu();
    assert_eq!(frame.mac.as_slice(), router);
    assert_eq!(npdu.destination.unwrap().mac_address.as_slice(), [0xFF]);
    client.stop().await.unwrap();
}

/// An empty router names no router, so it is refused, not read as the
/// local broadcast form.
#[tokio::test(start_paused = true)]
async fn an_empty_router_is_refused() {
    let (mut client, sent, _inbound) = client_on(
        DataLink::Mstp,
        &[1],
        no_groups(),
        ClientOptions::default(),
        0,
    )
    .await;
    let refused = client
        .confirmed_request(&Station::routed(&[], NETWORK, STATION), READ, &[0x0C])
        .await;
    assert!(refused_with(&refused, "names no router"), "{refused:?}");
    assert!(sent.is_empty());
    client.stop().await.unwrap();
}

/// The ComplexAck a router relays from `station` on `network` for the
/// request in `frame`.
fn relayed_ack(frame: &bacnet_transport::test_transport::SentFrame, router: &[u8]) -> ReceivedNpdu {
    let Apdu::ConfirmedRequest(request) = frame.apdu() else {
        panic!("a confirmed request");
    };
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
            service_ack: bytes::Bytes::from_static(&[0x21, 0x2A]),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source: Some(NpduAddress {
                network: NETWORK,
                mac_address: MacAddr::from_slice(STATION),
            }),
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(router),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// With no router known, a confirmed request reaches a station on another
/// network by one broadcast on this link whose NPDU names the station; the
/// answer a router relays completes it, whichever router that is. A
/// router that is a group address is no part of this form, so nothing of
/// #2722 refuses it, while the station's own MAC is still checked (#2723).
#[tokio::test(start_paused = true)]
async fn a_request_by_local_broadcast_names_its_station_and_takes_the_relayed_answer() {
    let mstp_broadcast = GroupDestinations::new(|mac| mac == [0xFF]);
    let (mut client, sent, inbound) = client_on(
        DataLink::Mstp,
        &[1],
        mstp_broadcast,
        ClientOptions::default(),
        1,
    )
    .await;
    let to = Station::routed_via_local_broadcast(NETWORK, STATION);
    let read = client.confirmed_request(&to, READ, &[0x0C]);
    let answer = async {
        while sent.is_empty() {
            tokio::task::yield_now().await;
        }
        let frame = sent.frame(0);
        assert!(frame.broadcast, "sent by the link's broadcast");
        let npdu = frame.decode_npdu();
        let dest = npdu.destination.as_ref().unwrap();
        assert_eq!(
            (dest.network, dest.mac_address.as_slice()),
            (NETWORK, STATION)
        );
        assert!(npdu.expecting_reply);
        assert!(matches!(frame.apdu(), Apdu::ConfirmedRequest(_)));
        inbound.send(relayed_ack(&frame, &[9])).await.unwrap();
    };
    let (answered, ()) = tokio::join!(read, answer);
    assert_eq!(answered.unwrap().as_ref(), [0x21, 0x2A]);
    assert_eq!(client.tsm.lock().pending_count(), 0);

    // Unanswered, each retry goes the same way.
    sent.clear();
    let result = client.confirmed_request(&to, READ, &[0x0C]).await;
    assert!(timed_out(&result), "{result:?}");
    assert_eq!(sent.broadcasts().len(), 2);
    assert!(sent.unicasts().is_empty());

    // The station's MAC is checked as on any routed form.
    let refused = client
        .confirmed_request(
            &Station::routed_via_local_broadcast(NETWORK, &[0xFF]),
            READ,
            &[0x0C],
        )
        .await;
    assert!(
        refused_with(&refused, "broadcast of an MS/TP network"),
        "{refused:?}"
    );
    client.stop().await.unwrap();
}

/// On this client's own network, once it knows the number, the station is
/// this link's, and the request goes to its MAC.
#[tokio::test(start_paused = true)]
async fn a_request_by_local_broadcast_to_this_network_goes_to_the_station() {
    let (mut client, sent, _inbound) = client_on(
        DataLink::Mstp,
        &[1],
        no_groups(),
        ClientOptions::default(),
        0,
    )
    .await;
    client
        .network
        .local_network_number()
        .publish(bacnet_types::network_number::NetworkNumber::configured(NETWORK).unwrap());
    let result = client
        .confirmed_request(
            &Station::routed_via_local_broadcast(NETWORK, STATION),
            READ,
            &[0x0C],
        )
        .await;
    assert!(timed_out(&result), "{result:?}");
    let [frame] = sent.frames().try_into().unwrap();
    assert!(!frame.broadcast);
    assert_eq!(frame.mac.as_slice(), STATION);
    assert!(frame.decode_npdu().destination.is_none());
    client.stop().await.unwrap();
}

#[path = "one_station_tests.rs"]
mod one_station;
