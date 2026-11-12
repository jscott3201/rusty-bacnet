//! The Ethernet frame tap (#1856): what the receive path and the send path
//! hand to it. All but the last test run on every OS, with the socket's
//! send injected. The last needs a Linux interface and `CAP_NET_RAW`; run it
//! as the direct transport checks in
//! `crates/bacnet-integration-tests/tests/ethernet_network_numbers/README.md`
//! are run, one isolated container with `BACNET_ETHERNET_INTERFACE` set.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::data_link::DataLink;

use super::egress::{ReplyFailures, send_tapped};
use super::ingress::receive_frame;
use super::*;
use crate::tap::TapDirection::{self, Inbound, Outbound};
use crate::tap::{FrameTap, FrameTapReceiver, TapPeer, TappedFrame};

const LOCAL: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];
const STATION: [u8; 6] = [0x02, 0, 0, 0, 0, 0x77];
/// An NPDU carrying Who-Is.
const WHO_IS: [u8; 4] = [0x01, 0x00, 0x10, 0x08];
/// The information field of an XID command.
const XID_INFO: [u8; 3] = [0x81, 0x01, 0x01];

/// A raw LLC frame to `destination` from `source` with `control`, carrying
/// `info`, padded to the 60-octet minimum as a sender pads it.
fn frame(destination: [u8; 6], source: [u8; 6], control: u8, info: &[u8]) -> Vec<u8> {
    let mut frame = destination.to_vec();
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&((LLC_HEADER_LEN + info.len()) as u16).to_be_bytes());
    frame.extend_from_slice(&[BACNET_LLC_DSAP, BACNET_LLC_SSAP, control]);
    frame.extend_from_slice(info);
    frame.resize(frame.len().max(14 + MIN_ETHERNET_PAYLOAD), 0);
    frame
}

/// The station at [`LOCAL`]'s receive path, recording what it sends.
#[derive(Default)]
struct Station {
    tap: FrameTap,
    group_source_drops: AtomicU64,
    reply_failures: ReplyFailures,
    sent: Vec<Vec<u8>>,
}

impl Station {
    fn receive(&mut self, data: &[u8]) -> Option<EthernetFrame> {
        let sent = &mut self.sent;
        receive_frame(
            data,
            &LOCAL,
            &self.tap,
            &self.group_source_drops,
            &self.reply_failures,
            |frame| {
                sent.push(frame.to_vec());
                Ok(frame.len())
            },
        )
    }
}

fn next(frames: &mut FrameTapReceiver) -> TappedFrame {
    frames.try_recv().expect("a tapped frame")
}

fn assert_tapped(frame: &TappedFrame, direction: TapDirection, peer: [u8; 6], bytes: &[u8]) {
    assert_eq!(
        (frame.direction, frame.link, frame.peer),
        (direction, DataLink::Ethernet, TapPeer::Ethernet(peer))
    );
    assert_eq!(frame.bytes[..], bytes[..], "the whole frame, padding too");
}

/// A frame the transport takes is tapped whole, as received, with its
/// sender as the peer, before the node acts on it: an XID or TEST command
/// shows before its response, which is tapped as sent. A UI frame to this
/// station or the broadcast is tapped and still handed up, and a frame from
/// a group source is tapped and still dropped and counted.
#[test]
fn every_frame_the_transport_takes_is_tapped_before_its_answer() {
    let mut station = Station::default();
    let mut frames = station.tap.subscribe();

    for destination in [LOCAL, ETHERNET_BROADCAST] {
        let ui = frame(destination, STATION, LLC_CONTROL_UI, &WHO_IS);
        let handed_up = station.receive(&ui).expect("a UI frame goes up");
        assert_eq!(handed_up.payload[..], WHO_IS);
        assert_tapped(&next(&mut frames), Inbound, STATION, &ui);
    }

    for (control, info) in [
        (LLC_CONTROL_XID_CMD, &XID_INFO[..]),
        (LLC_CONTROL_TEST_CMD, &WHO_IS[..]),
    ] {
        let command = frame(LOCAL, STATION, control, info);
        assert!(station.receive(&command).is_none());
        assert_tapped(&next(&mut frames), Inbound, STATION, &command);
        let response = station.sent.pop().expect("the command is answered");
        assert_tapped(&next(&mut frames), Outbound, STATION, &response);
    }

    let group = [0x01, 0x00, 0x5E, 0x00, 0x00, 0x42];
    let forged = frame(LOCAL, group, LLC_CONTROL_XID_CMD, &XID_INFO);
    assert!(station.receive(&forged).is_none());
    assert_tapped(&next(&mut frames), Inbound, group, &forged);
    assert_eq!(station.group_source_drops.load(Ordering::Relaxed), 1);
    assert!(station.sent.is_empty(), "a group source is never answered");
    assert!(frames.try_recv().is_none());
}

/// A frame the transport doesn't take is not tapped: one to another station,
/// one from this station's own MAC, and one too short to name its sender.
#[test]
fn a_frame_the_transport_ignores_is_not_tapped() {
    let mut station = Station::default();
    let mut frames = station.tap.subscribe();
    let elsewhere = frame([0x02, 0, 0, 0, 0, 0x55], STATION, LLC_CONTROL_UI, &WHO_IS);
    let own = frame(LOCAL, LOCAL, LLC_CONTROL_XID_CMD, &XID_INFO);
    for ignored in [&elsewhere[..], &own[..], &elsewhere[..11]] {
        assert!(station.receive(ignored).is_none());
    }
    assert!(station.sent.is_empty());
    assert!(frames.try_recv().is_none());
    assert_eq!(station.tap.counters().delivered, 0);
}

/// A sent frame is tapped whole, with its destination as the peer, before
/// the socket sends it (#2519): an answer the receive loop taps while the
/// send has yet to return comes after it. One that then fails, or goes
/// short, has been tapped too.
#[test]
fn a_frame_is_tapped_before_it_goes() {
    let tap = FrameTap::new();
    let mut frames = tap.subscribe();
    for destination in [STATION, ETHERNET_BROADCAST] {
        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &destination, &LOCAL, &WHO_IS);
        let mut tapped = None;
        let send = |f: &[u8]| {
            tapped = frames.try_recv();
            Ok(f.len())
        };
        send_tapped(&tap, &destination, &buf, send).unwrap();
        assert_tapped(&tapped.expect("tapped first"), Outbound, destination, &buf);
    }
    let unsent = frame(STATION, LOCAL, LLC_CONTROL_UI, &WHO_IS);
    let refused = |_: &[u8]| Err(io::Error::from(io::ErrorKind::OutOfMemory));
    assert!(send_tapped(&tap, &STATION, &unsent, refused).is_err());
    assert!(send_tapped(&tap, &STATION, &unsent, |f| Ok(f.len() - 1)).is_err());
    for _ in 0..2 {
        assert_tapped(&next(&mut frames), Outbound, STATION, &unsent);
    }
    assert!(frames.try_recv().is_none());
}

/// With nobody subscribed the receive path works as before and the tap
/// counts nothing.
#[test]
fn without_a_subscriber_frames_still_go_up_and_are_answered() {
    let mut station = Station::default();
    let ui = frame(LOCAL, STATION, LLC_CONTROL_UI, &WHO_IS);
    assert!(station.receive(&ui).is_some());
    let xid = frame(LOCAL, STATION, LLC_CONTROL_XID_CMD, &XID_INFO);
    assert!(station.receive(&xid).is_none());
    assert_eq!(station.sent.len(), 1);
    assert_eq!(station.tap.counters(), Default::default());
}

/// `AnyTransport` hands out the Ethernet transport's own tap.
#[cfg(target_os = "linux")]
#[test]
fn any_transport_reaches_the_ethernet_tap() {
    let transport = EthernetTransport::new("unused");
    let _frames = transport.frame_tap().subscribe();
    let any = crate::any::AnyTransport::Ethernet(transport);
    let tap = any.frame_tap().expect("Ethernet has a tap");
    assert_eq!(tap.counters().subscribers, 1);
}

/// On a real interface, the frames the transport sends and the frames it
/// takes reach the tap with their direction and peer: a unicast and a
/// broadcast it sends, a UI frame and an XID command it receives, and the XID
/// response, and an XID command from a group source, dropped unanswered.
/// Another station's frame sent before them is not taken.
///
/// The frames come from a raw socket on the same interface: the kernel hands
/// each frame it sends to the transport's socket too, as the group-source
/// fixture in `ethernet_network_numbers.rs` relies on. A bridge in hairpin
/// mode, as Docker's can be, also sends a frame for this station back to it,
/// and the transport takes that copy as well: the tap shows every copy it
/// takes, one per NPDU handed up and one per XID command answered.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires explicit isolated Linux interface and CAP_NET_RAW"]
async fn sent_and_received_frames_reach_the_tap_on_an_interface() {
    use crate::port::TransportPort;
    use std::time::Duration;

    let interface =
        std::env::var("BACNET_ETHERNET_INTERFACE").expect("explicit isolated interface");
    let mut transport = EthernetTransport::new(&interface);
    let mut frames = transport.frame_tap().subscribe();
    let mut npdus = transport.start().await.unwrap();
    let local: [u8; 6] = transport.local_mac().try_into().unwrap();

    transport.send_unicast(&WHO_IS, &STATION).await.unwrap();
    transport.send_broadcast(&WHO_IS).await.unwrap();
    for destination in [STATION, ETHERNET_BROADCAST] {
        let sent = frames.try_recv().expect("tapped as it went");
        let decoded = decode_ethernet_frame(&sent.bytes).unwrap();
        assert_tapped(&sent, Outbound, destination, &sent.bytes);
        assert_eq!((decoded.destination, decoded.source), (destination, local));
        assert_eq!(decoded.payload[..], WHO_IS);
    }

    // Bound to the interface's link address as the transport binds its own.
    let protocol = (libc::ETH_P_ALL as u16).to_be();
    let link = socket2::Socket::new(
        socket2::Domain::PACKET,
        socket2::Type::RAW,
        Some(socket2::Protocol::from(i32::from(protocol))),
    )
    .unwrap();
    let address = super::transport::interface_link(&interface).unwrap();
    nix::sys::socket::bind(std::os::fd::AsRawFd::as_raw_fd(&link), &address).unwrap();
    let elsewhere = frame([0x02, 0, 0, 0, 0, 0x55], STATION, LLC_CONTROL_UI, &WHO_IS);
    let ui = frame(local, STATION, LLC_CONTROL_UI, &WHO_IS);
    let xid = frame(local, STATION, LLC_CONTROL_XID_CMD, &XID_INFO);
    let group = [0x01, 0x00, 0x5E, 0x00, 0x00, 0x42];
    let forged = frame(local, group, LLC_CONTROL_XID_CMD, &XID_INFO);
    for raw in [&elsewhere, &ui, &xid, &forged] {
        assert_eq!(link.send(raw).unwrap(), raw.len());
    }

    // Everything tapped until nothing more comes for half a second.
    let mut tapped = Vec::new();
    while let Ok(next) = tokio::time::timeout(Duration::from_millis(500), frames.recv()).await {
        tapped.push(next.expect("the tap is open"));
    }
    let response = build_xid_response(&local, &STATION);
    let (mut uis, mut commands, mut responses, mut forgeries) = (0, 0, 0, 0);
    for frame in &tapped {
        let (count, direction, peer, bytes) = match frame.bytes[16] {
            _ if frame.bytes[..] == forged[..] => (&mut forgeries, Inbound, group, &forged),
            LLC_CONTROL_UI => (&mut uis, Inbound, STATION, &ui),
            LLC_CONTROL_XID_CMD => (&mut commands, Inbound, STATION, &xid),
            _ => (&mut responses, Outbound, STATION, &response),
        };
        assert_tapped(frame, direction, peer, bytes);
        *count += 1;
        assert!(responses <= commands, "a command shows before its answer");
    }
    let mut handed_up = 0;
    while let Ok(npdu) = npdus.try_recv() {
        let from = (&npdu.source_mac[..], &npdu.npdu[..]);
        assert_eq!(from, (&STATION[..], &WHO_IS[..]));
        handed_up += 1;
    }
    assert!(uis >= 1 && commands >= 1 && forgeries >= 1, "{tapped:?}");
    assert_eq!((uis, responses), (handed_up, commands), "{tapped:?}");
    assert_eq!(forgeries, transport.group_source_drops());

    transport.stop().await.unwrap();
    assert!(frames.try_recv().is_none());
}
