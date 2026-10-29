//! A confirmed request to a broadcast or group address (#1479).
//!
//! With no DNET, a destination that reaches a group of nodes is a local
//! broadcast, which Clause 6.3 keeps for Unconfirmed-Request PDUs. The
//! client refuses a confirmed request to one before any transaction state
//! exists, including a routed target that names its own network and so
//! goes local; an unconfirmed request still goes, and so does a routed
//! request whose link DA is the broadcast MAC, since its DNET/DADR name one
//! device.
use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{GroupDestinations, ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::network_number::NetworkNumber;
use std::net::Ipv4Addr;
use tokio::sync::mpsc;

use super::{BACnetClient, ClientConfig};

const CLIENT: [u8; 1] = [1];
const PEER: [u8; 1] = [2];
/// The link's broadcast MAC, as on MS/TP.
const BROADCAST: [u8; 1] = [0xFF];

#[tokio::test(start_paused = true)]
async fn a_confirmed_request_to_the_broadcast_mac_is_refused_before_any_state() {
    let (mut transport, mut peer) = LoopbackTransport::pair(CLIENT.to_vec(), PEER.to_vec());
    transport.set_broadcast_mac(BROADCAST.to_vec());
    let mut frames = peer.start().await.unwrap();
    let config = ClientConfig {
        apdu_timeout_ms: 20,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, transport).await.unwrap();

    let refused = client
        .confirmed_request(&BROADCAST, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
        .await;
    assert!(
        matches!(&refused, Err(Error::Encoding(message))
            if message.contains("not to a broadcast or group address")),
        "{refused:?}"
    );
    assert!(
        frames.try_recv().is_err(),
        "a refused request reached the link"
    );
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);

    // An unconfirmed request may be broadcast this way.
    client
        .unconfirmed_request(&BROADCAST, UnconfirmedServiceChoice::WHO_IS, &[])
        .await
        .unwrap();
    let who_is = decode_npdu(frames.try_recv().unwrap().npdu).unwrap();
    assert!(matches!(
        decode_apdu(who_is.payload),
        Ok(Apdu::UnconfirmedRequest(_))
    ));

    // A routed request names one device with its DNET/DADR, so its link DA
    // may be the broadcast MAC; it goes out and its transaction times out
    // unanswered.
    let routed = client
        .confirmed_request_routed(
            &BROADCAST,
            5,
            &[7],
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x0C],
        )
        .await;
    assert!(
        matches!(routed, Err(Error::Remote(RemoteError::Abort { reason }))
            if reason == AbortReason::TSM_TIMEOUT),
        "{routed:?}"
    );
    let request = decode_npdu(frames.try_recv().unwrap().npdu).unwrap();
    assert!(matches!(
        decode_apdu(request.payload),
        Ok(Apdu::ConfirmedRequest(_))
    ));

    client.stop().await.unwrap();
}

/// A loopback link whose group destinations follow another transport's
/// rule, so the client's check meets each B/IP or B/IPv6 address form
/// while every send it does make is still seen.
struct GroupRuleLink {
    link: LoopbackTransport,
    groups: GroupDestinations,
}

impl TransportPort for GroupRuleLink {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.link.start().await
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.link.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.link.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.link.send_broadcast(npdu).await
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.link.local_receive_apdu_capacity()
    }

    fn local_mac(&self) -> &[u8] {
        self.link.local_mac()
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.groups.contains(mac)
    }
}

async fn refuses_each_group_form(groups: GroupDestinations, forms: &[Vec<u8>], unicast: &[u8]) {
    let (link, mut peer) = LoopbackTransport::pair(CLIENT.to_vec(), PEER.to_vec());
    let mut frames = peer.start().await.unwrap();
    let config = ClientConfig {
        apdu_timeout_ms: 20,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, GroupRuleLink { link, groups })
        .await
        .unwrap();
    for mac in forms {
        let refused = client
            .confirmed_request(mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
            .await;
        assert!(
            matches!(&refused, Err(Error::Encoding(message))
                if message.contains("not to a broadcast or group address")),
            "{mac:02X?}: {refused:?}"
        );
    }
    assert!(
        frames.try_recv().is_err(),
        "a refused request reached the link"
    );
    assert_eq!(client.tsm.lock().await.pending_count(), 0);

    // One device's address still takes a confirmed request.
    let sent = client
        .confirmed_request(unicast, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
        .await;
    assert!(
        matches!(sent, Err(Error::Remote(RemoteError::Abort { reason }))
            if reason == AbortReason::TSM_TIMEOUT),
        "{sent:?}"
    );
    assert!(frames.try_recv().is_ok());
    client.stop().await.unwrap();
}

/// Every B/IP group form: the limited broadcast and the configured
/// broadcast IP at this port and another, and IPv4 multicast.
#[tokio::test(start_paused = true)]
async fn a_confirmed_request_to_any_bip_group_address_is_refused() {
    let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0xBAC0, Ipv4Addr::new(192, 168, 1, 255));
    let forms = [
        vec![255, 255, 255, 255, 0xBA, 0xC0],
        vec![255, 255, 255, 255, 0xBA, 0xC1],
        vec![192, 168, 1, 255, 0xBA, 0xC0],
        vec![192, 168, 1, 255, 0xBA, 0xC1],
        vec![224, 0, 0, 1, 0xBA, 0xC0],
        vec![239, 255, 255, 250, 0x07, 0x6C],
    ];
    refuses_each_group_form(
        bip.group_destinations(),
        &forms,
        &[192, 168, 1, 7, 0xBA, 0xC0],
    )
    .await;
}

/// A B/IPv6 MAC is the peer's 3-octet VMAC (U.1.3), which never names a
/// group: a B/IPv6 broadcast is the zero-length MAC. An 18-octet address,
/// an IPv6 multicast group's or a node's, is refused by the transport before
/// it starts any resolution, and the client keeps no transaction for it
/// (#1637).
#[cfg(feature = "ipv6")]
#[tokio::test]
async fn a_confirmed_request_to_an_18_octet_bip6_address_is_refused() {
    use bacnet_transport::bip6::{Bip6ResolutionCounters, Bip6Transport};
    use std::net::Ipv6Addr;

    let mac = |ip: &str, port: u16| {
        let ip: Ipv6Addr = ip.parse().unwrap();
        [&ip.octets()[..], &port.to_be_bytes()].concat()
    };
    let forms = [
        mac("ff02::bac0", 0xBAC0),
        mac("ff02::1", 0xBAC0),
        mac("ff05::1:3", 0x1234),
        mac("::1", 0xBAC0),
    ];
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    let config = ClientConfig {
        apdu_timeout_ms: 20,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, transport).await.unwrap();
    for mac in &forms {
        assert!(!client.transport().is_group_destination(mac));
        let refused = client
            .confirmed_request(mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
            .await;
        assert!(
            matches!(&refused, Err(Error::Transport(error))
                if error.kind() == std::io::ErrorKind::InvalidInput),
            "{mac:02X?}: {refused:?}"
        );
    }
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert_eq!(
        client.transport().resolution_counters(),
        Bip6ResolutionCounters::default(),
        "no resolution started"
    );
    client.stop().await.unwrap();
}

/// A routed target naming the client's own network goes local (#1358), so
/// one whose DADR is the link's broadcast MAC is a local broadcast and is
/// refused like a direct one, before any state.
#[tokio::test(start_paused = true)]
async fn a_routed_request_to_this_networks_broadcast_mac_is_refused() {
    let (mut transport, mut peer) = LoopbackTransport::pair(CLIENT.to_vec(), PEER.to_vec());
    transport.set_broadcast_mac(BROADCAST.to_vec());
    let mut frames = peer.start().await.unwrap();
    let mut client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    let number: &LocalNetworkNumber = client.network.local_network_number();
    number.publish(NetworkNumber::configured(5).unwrap());

    let refused = client
        .confirmed_request_routed(
            &PEER,
            5,
            &BROADCAST,
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x0C],
        )
        .await;
    assert!(
        matches!(&refused, Err(Error::Encoding(message))
            if message.contains("not to a broadcast or group address")),
        "{refused:?}"
    );
    assert!(
        frames.try_recv().is_err(),
        "a refused request reached the link"
    );
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
}
