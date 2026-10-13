//! What the router scenario reports in a good run. `tests/router.rs`
//! compares the native run's summary with it, and the emulation build of the
//! `router` image the emulated CPU's, so the two can't drift apart.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::BvlcFunction;
use bacnet_mstp_codec::FrameType;
use bacnet_router_core::message::{MessageType, RejectReason};
use bacnet_router_core::router::{Counters, Event, Kind};
use bacnet_router_core::{Duration, PortId};

use super::{
    Outcome, SentDatagram, SentFrame, Summary, DATAGRAMS, EVENTS, FRAMES, NODE, PEER_BBMD,
    PEER_STATION, RECEIVES, SENDS, TARGET_STATION,
};
use crate::tally::Sent;

/// The hash over every datagram the B/IP port sends and where it goes. It
/// was worked out apart from the codecs, from the 11 datagrams written out
/// by hand from the Annex J.2 and Clause 6.2 layouts (158 octets).
const BIP_FNV: u32 = 0xA941_5489;

/// The hash over every frame the MS/TP core sends. It was worked out apart
/// from the codecs, from the 16 frames written out by hand from the Clause
/// 9.3 and 6.2 layouts (1,704 octets), with the CRCs of Clause 9.6 and
/// Annex G, and for the extended frame the COBS encoding of Clause 9.10 and
/// Annex T; that code reproduces Annex T.3's whole example frame.
const MSTP_FNV: u32 = 0xDE4B_F63F;

/// The subnet's broadcast address.
const SUBNET_BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0);

/// Every slot of `N` from `filled`, in order, the rest unused.
const fn slots<T: Copy, const M: usize, const N: usize>(filled: [T; M]) -> [Option<T>; N] {
    let mut slots = [None; N];
    let mut index = 0;
    while index < M {
        slots[index] = Some(filled[index]);
        index += 1;
    }
    slots
}

/// What the router took, step by step: everything routed, answered or
/// relayed, but step 8's global broadcast, which also reaches its own
/// application.
const RECEIVED: [Option<Outcome>; RECEIVES] = slots([
    Outcome::Handled, // 2: for station 5
    Outcome::Handled, // 3: the peer's forward
    Outcome::Handled, // 4: station 2's request
    Outcome::Handled, // 5: for network 9
    Outcome::Handled, // 5: again
    Outcome::Handled, // 6: too long
    Outcome::Handled, // 6: the longest that fits
    Outcome::Handled, // 7: Who-Is-Router
    Outcome::Handled, // 7: the table query
    Outcome::Local {
        group: true,
        global: true,
    }, // 8: the global broadcast
]);

const B: u8 = 0;
const M: u8 = 1;

/// The sends the router asked for: port 0 is B/IP and port 1 MS/TP; `true`
/// for every node there.
const SENT: [Option<(u8, Kind, bool)>; SENDS] = slots([
    // 1: on each port in turn, its number, then the network beyond it.
    (B, Kind::Announce(MessageType::NETWORK_NUMBER_IS), true),
    (B, Kind::Announce(MessageType::I_AM_ROUTER_TO_NETWORK), true),
    (M, Kind::Announce(MessageType::NETWORK_NUMBER_IS), true),
    (M, Kind::Announce(MessageType::I_AM_ROUTER_TO_NETWORK), true),
    // 2, 3 and 4.
    (M, Kind::Forward, false),
    (M, Kind::Forward, false),
    (B, Kind::Forward, false),
    // 5: the search on every other port, then the reject; the second time,
    // within the cooldown, the reject alone.
    (M, Kind::Solicit, true),
    (B, Kind::Reject(RejectReason::NOT_DIRECTLY_CONNECTED), false),
    (B, Kind::Reject(RejectReason::NOT_DIRECTLY_CONNECTED), false),
    // 6: the reject for the one too long, which goes nowhere else, then
    // the one that fits.
    (B, Kind::Reject(RejectReason::MESSAGE_TOO_LONG), false),
    (M, Kind::Forward, false),
    // 7: each answer to its asker: the I-Am-Router on its port, the Ack to
    // station 2.
    (M, Kind::Answer(MessageType::I_AM_ROUTER_TO_NETWORK), true),
    (
        M,
        Kind::Answer(MessageType::INITIALIZE_ROUTING_TABLE_ACK),
        false,
    ),
    // 8.
    (B, Kind::Global, true),
]);

/// Step 6's NPDU of 1494 octets would leave at 1498 on MS/TP, whose limit
/// is 1497; the reject names network 2.
const EVENTS_SEEN: [Option<Event>; EVENTS] = slots([Event::TooLong {
    port: PortId(1),
    octets: 1498,
    limit: 1497,
    rejected: Some(2),
}]);

const UNICAST: BvlcFunction = BvlcFunction::ORIGINAL_UNICAST_NPDU;
const BROADCAST: BvlcFunction = BvlcFunction::ORIGINAL_BROADCAST_NPDU;
const FORWARDED: BvlcFunction = BvlcFunction::FORWARDED_NPDU;

/// The datagrams the B/IP port sent, with their lengths: the 4-octet BVLL
/// header, 6 more for a Forwarded-NPDU's origin, then the NPDU.
const DATAGRAMS_SENT: [Option<SentDatagram>; DATAGRAMS] = slots([
    // 1: the Network-Number-Is (6) and I-Am-Router (5), each on the subnet
    // and forwarded to the peer as the BBMD's own.
    (BROADCAST, SUBNET_BROADCAST, 10),
    (FORWARDED, PEER_BBMD, 16),
    (BROADCAST, SUBNET_BROADCAST, 9),
    (FORWARDED, PEER_BBMD, 15),
    // 3: the peer's forward (9) on the subnet, which the peer couldn't
    // reach itself.
    (FORWARDED, SUBNET_BROADCAST, 19),
    // 4: the request with SNET 2 and SADR 2, without DNET, DADR and hop
    // count (17).
    (UNICAST, NODE, 21),
    // 5 and 6: three rejects (6).
    (UNICAST, NODE, 10),
    (UNICAST, NODE, 10),
    (UNICAST, NODE, 10),
    // 8: the global broadcast with SNET 2, SADR 2 and its hop count one
    // lower (12), on the subnet and to the peer.
    (BROADCAST, SUBNET_BROADCAST, 16),
    (FORWARDED, PEER_BBMD, 22),
]);

const DATA: FrameType = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;
const TOKEN: SentFrame = (FrameType::TOKEN, PEER_STATION, 8);
const ALL: u8 = 255;

/// The frames the MS/TP core sent, with their lengths: 8 octets of
/// preamble and header, then the NPDU and 2 of data CRC.
const FRAMES_SENT: [Option<SentFrame>; FRAMES] = slots([
    // 1: the Network-Number-Is (6) and I-Am-Router (5) on the first token;
    // then, with no successor known yet, a poll that station 2 answers,
    // and the token to it.
    (DATA, ALL, 16),
    (DATA, ALL, 15),
    (FrameType::POLL_FOR_MASTER, PEER_STATION, 8),
    TOKEN,
    // 2 and 3: each Who-Is (13, with SNET 1 and a six-octet SADR) on its
    // token.
    (DATA, TARGET_STATION, 23),
    TOKEN,
    (DATA, TARGET_STATION, 23),
    TOKEN,
    // 4: Reply Postponed to station 2's request.
    (FrameType::REPLY_POSTPONED, PEER_STATION, 8),
    // 5: the Who-Is-Router for network 9 (5).
    (DATA, ALL, 15),
    TOKEN,
    // 6: 1497 octets in a Type 33 frame: 1498 COBS-encoded, then the five
    // of the encoded CRC-32K.
    (
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        TARGET_STATION,
        1511,
    ),
    TOKEN,
    // 7: the I-Am-Router for network 1 (5) and the Ack (12), on one token.
    (DATA, ALL, 15),
    (DATA, PEER_STATION, 22),
    TOKEN,
]);

/// The summary [`run`](super::run) returns in a good run.
#[must_use]
pub fn expected() -> Summary {
    // `Counters` is `#[non_exhaustive]`, so it is built from its default.
    let mut counters = Counters::default();
    // Two of reason 1 for network 9, one of reason 4 for step 6.
    counters.rejects = 3;
    // One search: the second message for network 9 came within 5 s.
    counters.solicitations = 1;
    counters.egress_too_long = 1;
    Summary {
        received: RECEIVED,
        sends: SENT,
        events: EVENTS_SEEN,
        counters,
        // The two direct routes: network 9 was never found.
        routes: 2,
        datagrams: DATAGRAMS_SENT,
        bip: Sent {
            count: 11,
            octets: 158,
            fnv: BIP_FNV,
        },
        bip_refused: 0,
        frames: FRAMES_SENT,
        mstp: Sent {
            count: 16,
            octets: 1704,
            fnv: MSTP_FNV,
        },
        // Six tokens, the answer to the poll, and four frames with NPDUs:
        // the request, the two route questions and the global broadcast.
        frames_received: 11,
        frames_transmitted: 16,
        npdus_delivered: 4,
        parser_errors: 0,
        // For one station: steps 2, 3 and 6, and the Ack; for all: the
        // two announcements, the search and the I-Am-Router.
        queued: (4, 4),
        // T_turnaround at 9600 baud, 40 bit times rounded up to a whole
        // microsecond: Reply Postponed goes as soon as the line allows,
        // not at the core's own decision deadline, 225 ms on.
        postponed_after: Some(Duration::from_micros(4167)),
        errors: 0,
    }
}
