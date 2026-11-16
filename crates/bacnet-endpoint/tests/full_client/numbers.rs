//! The local network number on a node with a full client: one owner. The
//! client asks for the number, the node's owner answers for it, and neither
//! does the other's part, whichever started first.
use super::*;
use bacnet_encoding::apdu::UnconfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_npdu::NetworkMessageType;
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::UnconfirmedServiceChoice;

/// A router on the link, which announces network 5 and relays for it.
const ROUTER: u8 = 9;
const NETWORK: u16 = 5;
/// A device the router relays an I-Am for, and its MAC on network 5.
const RELAYED_DEVICE: u32 = 777;
const RELAYED_MAC: [u8; 1] = [0x09];

/// How many of `message` the node has sent.
fn sent(lab: &Lab, message: NetworkMessageType) -> usize {
    network_messages(&lab.node_sent)
        .into_iter()
        .filter(|sent| *sent == message)
        .count()
}

/// The router's Network-Number-Is for network 5, as a local broadcast.
fn announce_the_number(lab: &Lab) {
    let [high, low] = NETWORK.to_be_bytes();
    let npdu = Bytes::from(vec![0x01, 0x80, 0x13, high, low, 0x01]);
    lab.wire.broadcast(&Wire::mac(ROUTER), npdu);
}

/// A What-Is-Network-Number from node [`OTHER`], as a local broadcast.
fn ask_the_number(lab: &Lab) {
    let npdu = Bytes::from_static(&[0x01, 0x80, 0x12]);
    lab.wire.broadcast(&Wire::mac(OTHER), npdu);
}

/// An I-Am from the relayed device, which the router passes on with network
/// 5 and the device's MAC there as its source.
fn relay_an_i_am(lab: &Lab) {
    let mut parameters = BytesMut::new();
    IAmRequest {
        object_identifier: oid(ObjectType::DEVICE, RELAYED_DEVICE),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 42,
    }
    .encode(&mut parameters);
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: parameters.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source: Some(NpduAddress {
                network: NETWORK,
                mac_address: MacAddr::from_slice(&RELAYED_MAC),
            }),
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    lab.wire.broadcast(&Wire::mac(ROUTER), npdu.freeze());
}

/// The relayed device's row in `client`'s table, once it has one.
async fn relayed_row(
    client: &BACnetClient<TestTransport>,
) -> bacnet_client::discovery::DiscoveredDevice {
    eventually(WAIT, "the relayed device's row", async || {
        client.get_device(RELAYED_DEVICE).await.unwrap().is_some()
    })
    .await;
    client.get_device(RELAYED_DEVICE).await.unwrap().unwrap()
}

/// A What-Is-Network-Number broadcast gets one Network-Number-Is from the
/// node, from its one owner, however long the link is watched.
async fn assert_one_answer(lab: &Lab) {
    let before = sent(lab, NetworkMessageType::NETWORK_NUMBER_IS);
    ask_the_number(lab);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(sent(lab, NetworkMessageType::NETWORK_NUMBER_IS), before + 1);
}

#[tokio::test(start_paused = true)]
async fn a_client_started_before_the_number_is_known_asks_once_and_takes_the_owners_number() {
    for _ in 0..20 {
        let lab = lab_with(SessionRole::Both, true).await;
        let client = lab.session.full_client().unwrap();
        assert_eq!(client.network_number(), None);
        // Nothing asks before the first need. The first Who-Is asks once,
        // and the session's owner asks nothing of its own.
        assert_eq!(sent(&lab, NetworkMessageType::WHAT_IS_NETWORK_NUMBER), 0);
        client.who_is(Broadcast::Global, None).await.unwrap();
        assert_eq!(sent(&lab, NetworkMessageType::WHAT_IS_NETWORK_NUMBER), 1);
        // Nothing on this link answers the question, as on a site with no
        // router that knows its number. The Who-Is went anyway, and later
        // ones neither wait nor ask again.
        tokio::time::sleep(Duration::from_secs(60)).await;
        client.who_is(Broadcast::Global, None).await.unwrap();
        client
            .who_is_directed(&Wire::mac(PEER), None)
            .await
            .unwrap();
        assert_eq!(sent(&lab, NetworkMessageType::WHAT_IS_NETWORK_NUMBER), 1);
        assert_eq!(sent(&lab, NetworkMessageType::NETWORK_NUMBER_IS), 0);
        assert!(client.get_device(PEER_DEVICE).await.unwrap().is_some());

        // A device the router relays is a routed row while the number is
        // unknown.
        relay_an_i_am(&lab);
        let routed = relayed_row(&client).await;
        assert_eq!(routed.source_network, Some(NETWORK));

        // The node's owner learns the number, and the client's table
        // follows it: the relayed device is on this network, at its own MAC.
        announce_the_number(&lab);
        eventually(WAIT, "the relayed device made local", async || {
            let row = client.get_device(RELAYED_DEVICE).await.unwrap();
            row.is_some_and(|row| row.source_network.is_none())
        })
        .await;
        assert_eq!(client.network_number(), Some(NETWORK));
        assert_eq!(
            relayed_row(&client).await.mac_address.as_slice(),
            RELAYED_MAC
        );

        assert_one_answer(&lab).await;
        assert_eq!(sent(&lab, NetworkMessageType::WHAT_IS_NETWORK_NUMBER), 1);
        drop(client);
        stop(lab).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_client_started_after_the_number_is_known_asks_nothing() {
    for _ in 0..20 {
        let lab = lab_with(SessionRole::Both, true).await;
        // The session alone answers the question once it has the number.
        announce_the_number(&lab);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_one_answer(&lab).await;

        let client = lab.session.full_client().unwrap();
        assert_eq!(client.network_number(), Some(NETWORK));
        client.who_is(Broadcast::Global, None).await.unwrap();
        // A relayed device from this network is local from its first I-Am.
        relay_an_i_am(&lab);
        assert_eq!(relayed_row(&client).await.source_network, None);
        // Still one answer per question with the client there, and the
        // client, which knows the number, never asked for it.
        assert_one_answer(&lab).await;
        assert_eq!(sent(&lab, NetworkMessageType::WHAT_IS_NETWORK_NUMBER), 0);

        drop(client);
        stop(lab).await;
    }
}

#[tokio::test(start_paused = true)]
async fn on_a_link_with_no_number_controls_nothing_is_asked_or_answered() {
    let lab = lab_with(SessionRole::Both, false).await;
    let client = lab.session.full_client().unwrap();
    client.who_is(Broadcast::Global, None).await.unwrap();
    ask_the_number(&lab);
    announce_the_number(&lab);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(network_messages(&lab.node_sent), []);
    assert_eq!(client.network_number(), None);
    // The router's announcement still reached the client, as a report.
    let mut reports = client.network_reports();
    announce_the_number(&lab);
    tokio::time::timeout(WAIT, reports.recv())
        .await
        .expect("a network report")
        .unwrap();

    drop(client);
    stop(lab).await;
}
