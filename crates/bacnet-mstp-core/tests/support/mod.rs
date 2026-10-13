//! Helpers shared by the pending-send and transmit-start tests. Each test
//! binary uses a subset.

#![allow(dead_code)]

use bacnet_mstp_codec::{decode_frame, Frame, FrameHeader, FrameType};
use bacnet_mstp_core::{
    Action, Direction, Duration, HeaplessQueues, Instant, MasterCore, MasterState, MstpCoreConfig,
    NetworkPriority, TransmitId,
};

pub type Queues = HeaplessQueues<4, 4>;
pub type Actions = bacnet_mstp_core::ActionBuffers;
pub type Core<'rx> = MasterCore<'rx, Queues>;

/// This station.
pub const STATION: u8 = 1;
/// Its peer, which becomes its successor.
pub const PEER: u8 = 2;

/// Station 1 of masters 0 to 3 at 9600 baud: T_turnaround 4167 µs, one
/// octet 1042 µs.
pub fn config() -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = 3;
    config
}

/// Station 1's core over `rx` and `actions`, from time zero.
pub fn new_core<'a, const DELIVERIES: usize>(
    rx: &'a mut [u8],
    actions: &'a mut bacnet_mstp_core::ActionBuffers<DELIVERIES>,
) -> Core<'a> {
    Core::new(config(), Queues::new(), rx, actions, Instant::ZERO).unwrap()
}

pub fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1_000)
}

pub fn us(value: u64) -> Duration {
    Duration::from_micros(value)
}

pub fn turnaround(core: &Core<'_>) -> Duration {
    core.config().t_turnaround
}

/// Feed a frame as a host assembler would.
pub fn validated(
    core: &mut Core<'_>,
    kind: FrameType,
    destination: u8,
    source: u8,
    data: &[u8],
    now: Instant,
) {
    core.on_validated_frame(Frame::new(kind, destination, source, data), now)
        .unwrap();
}

/// Feed `octets` one octet time apart from `first`; returns when the last
/// one arrived.
pub fn feed(core: &mut Core<'_>, octets: &[u8], first: Instant) -> Instant {
    let octet = core.config().octet_time();
    let mut at = first;
    for (index, &byte) in octets.iter().enumerate() {
        if index > 0 {
            at += octet;
        }
        core.on_octet(byte, at).unwrap();
    }
    at
}

/// The wire octets of a frame.
pub fn encode(kind: FrameType, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let mut wire = [0u8; 600];
    let length = Frame::new(kind, destination, source, data)
        .encode_into(&mut wire)
        .unwrap();
    wire[..length].to_vec()
}

/// Poll the direction change and the transmit, without starting it.
pub fn handed_over(core: &mut Core<'_>) -> (TransmitId, FrameHeader) {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    match core.poll_action() {
        Action::Transmit { id, frame } => (id, decode_frame(frame).unwrap().0.header),
        other => panic!("expected a transmit, got {other:?}"),
    }
}

/// Assert that the next action switches back to receive, and nothing
/// follows it.
pub fn expect_receive_only(core: &mut Core<'_>) {
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.poll_action(), Action::Idle);
}

/// Hand over, start at `now` and complete one millisecond later.
pub fn send(core: &mut Core<'_>, now: Instant) -> FrameHeader {
    let (id, header) = handed_over(core);
    core.on_transmit_started(id, now).unwrap();
    core.on_transmit_complete(id, now + Duration::from_millis(1))
        .unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    header
}

/// Service the next deadline and send the frame it hands over.
pub fn send_next(core: &mut Core<'_>) -> (Instant, FrameHeader) {
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    (due, send(core, due))
}

pub fn is(header: FrameHeader, kind: FrameType, destination: u8) -> bool {
    header.frame_type == kind && header.destination == destination
}

/// Station 1 takes a token from 2 at 10 ms with nothing queued. It knows no
/// successor, so it polls station 2; returns with that poll waiting out its
/// turnaround.
pub fn poll_waiting(core: &mut Core<'_>) {
    validated(core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.next_deadline(), Some(ms(10) + turnaround(core)));
}

/// Station 1 polls 2, which answers at once; returns with the Token for 2
/// waiting out its turnaround after that answer, which arrived at the
/// returned instant.
pub fn pass_waiting(core: &mut Core<'_>) -> Instant {
    poll_waiting(core);
    let (_, header) = send_next(core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    let answered = core.next_deadline().unwrap() - Duration::from_millis(15);
    validated(
        core,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        STATION,
        PEER,
        &[],
        answered,
    );
    assert_eq!(core.state(), MasterState::PassToken);
    answered
}

/// Station 1 passes the token to 2, hears nothing for T_usage_timeout and
/// hands its Token retry to the adapter. Returns when the retry was handed
/// over, and its id.
pub fn retry_handed_over(core: &mut Core<'_>) -> (Instant, TransmitId) {
    pass_waiting(core);
    let (_, header) = send_next(core);
    assert!(is(header, FrameType::TOKEN, PEER));
    let timeout = core.next_deadline().unwrap();
    core.on_deadline(timeout).unwrap();
    let (id, header) = handed_over(core);
    assert!(is(header, FrameType::TOKEN, PEER));
    assert_eq!(core.counters().token_retries, 1);
    (timeout, id)
}

/// A fresh station 1 loses the token and generates one in its slot, 510 ms
/// in; returns with the generating poll handed over, and its id.
pub fn generation_handed_over(core: &mut Core<'_>) -> TransmitId {
    core.on_deadline(ms(500)).unwrap();
    assert_eq!(core.state(), MasterState::NoToken);
    core.on_deadline(ms(510)).unwrap();
    let (id, header) = handed_over(core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    assert_eq!(core.counters().token_regenerations, 1);
    id
}

/// Queue an NPDU for the peer; `expecting` sets its data-expecting-reply bit.
pub fn enqueue(core: &mut Core<'_>, expecting: bool, now: Instant) {
    let control = if expecting { 0x04 } else { 0x00 };
    core.enqueue_npdu(PEER, &[0x01, control, 0x07], NetworkPriority::Normal, now)
        .unwrap();
}

/// Station 1 with successor 3 (station 2 is silent) runs through Npoll
/// token uses until it polls station 2 for maintenance; returns once that
/// poll has gone out.
pub fn maintenance_poll_sent(core: &mut Core<'_>) {
    poll_waiting(core);
    send_next(core);
    let timeout = core.next_deadline().unwrap();
    core.on_deadline(timeout).unwrap();
    let header = send(core, timeout);
    assert!(is(header, FrameType::POLL_FOR_MASTER, 3));
    let answered = timeout + Duration::from_millis(5);
    validated(
        core,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        STATION,
        3,
        &[],
        answered,
    );
    for _ in 0..=50 {
        let (passed, header) = send_next(core);
        assert!(is(header, FrameType::TOKEN, 3));
        // Station 3 uses the token, then passes it back.
        let used = passed + Duration::from_millis(3);
        validated(
            core,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            0,
            3,
            &[1, 0, 7],
            used,
        );
        let back = used + Duration::from_millis(2);
        validated(core, FrameType::TOKEN, STATION, 3, &[], back);
        if core.state() == MasterState::PollForMaster {
            let (_, header) = send_next(core);
            assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
            return;
        }
    }
    panic!("no maintenance poll after Npoll token uses");
}

/// Station 1 takes the token from 2 with a data-expecting-reply NPDU queued
/// for it and sends it. Returns when that request went out, with the core
/// waiting for the reply.
pub fn waiting_for_reply(core: &mut Core<'_>) -> Instant {
    enqueue(core, true, ms(0));
    validated(core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
    let (due, header) = send_next(core);
    assert!(is(header, FrameType::BACNET_DATA_EXPECTING_REPLY, PEER));
    assert_eq!(core.state(), MasterState::WaitForReply);
    due + Duration::from_millis(1)
}

/// The eight octets of a header whose CRC is wrong. A station's own
/// decoder counts it invalid at the last one, wherever it was going.
pub fn bad_header() -> Vec<u8> {
    let mut wire = encode(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, PEER, &[1, 2]);
    wire.truncate(8);
    wire[7] ^= 0x01;
    wire
}

/// Report one invalid frame whose first octet arrives at `first`, through
/// the core's own decoder or as a host assembler would: its octets, then
/// the verdict. Returns when it was found, at its last octet.
pub fn invalid_frame(core: &mut Core<'_>, host: bool, first: Instant) -> Instant {
    if host {
        let octet = core.config().octet_time();
        let last = first + us(7 * octet.as_micros());
        core.on_host_activity(8, last).unwrap();
        core.on_invalid_frame(last).unwrap();
        last
    } else {
        feed(core, &bad_header(), first)
    }
}
