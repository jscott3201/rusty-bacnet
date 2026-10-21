//! The core part: one token exchange with a peer, driven the way a UART
//! adapter would drive it, then the capacity paths.

use bacnet_mstp_codec::{decode_frame, Frame, FrameType};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Duration, Instant, MasterCore, MstpCoreConfig, QueueRefusal, ReplyDecision, RequestId,
    TransmitId,
};

use super::{Core, Storage, Summary, UNICAST_SLOTS, WHO_IS};

/// This station.
const STATION: u8 = 1;
/// The peer, which hands this station the token.
const PEER: u8 = 2;

/// Station 1 of masters 0-2, at the standard 9600 baud.
const CONFIG: MstpCoreConfig = {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = 2;
    config
};

/// The peer's ReadProperty of Device 1's Object_Name; the NPDU control
/// octet asks for a reply.
const READ_PROPERTY: [u8; 13] = [
    0x01, 0x04, 0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// This station's answer, a ComplexACK with the name "MSTP".
const COMPLEX_ACK: [u8; 21] = [
    0x01, 0x00, 0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D, 0x3E, 0x75, 0x05, 0x00,
    b'M', b'S', b'T', b'P', 0x3F,
];
/// The most actions one stimulus may produce before the scenario counts an
/// error and moves on, so a runaway core can't hang it.
const MAX_ACTIONS: usize = 16;

/// Injected time, advanced by the octets on the line and by waits.
struct Clock {
    now: Instant,
    octet: Duration,
}

impl Clock {
    /// Let `octets` octet times pass.
    fn pass(&mut self, octets: usize) {
        let octets = u32::try_from(octets).unwrap_or(u32::MAX);
        self.now += self.octet.saturating_mul(octets);
    }
}

/// Build the core over `storage`, run the token exchange, then the capacity
/// paths, offering `oversize` as the NPDU too long for a standard frame.
pub(crate) fn run(storage: &mut Storage, wire: &mut [u8], oversize: &[u8], summary: &mut Summary) {
    let mut clock = Clock {
        now: Instant::ZERO,
        octet: CONFIG.octet_time(),
    };
    let Ok(mut core) = MasterCore::new(CONFIG, storage, clock.now) else {
        summary.error();
        return;
    };
    let core = &mut core;

    // Something to send while holding the token.
    if core
        .enqueue_npdu(PEER, &WHO_IS, NetworkPriority::Normal, clock.now)
        .is_err()
    {
        summary.error();
    }
    // The peer passes the token: send the queued Who-Is, then, with no
    // successor known yet, poll the next station for a master.
    receive(
        core,
        wire,
        Frame::new(FrameType::TOKEN, STATION, PEER, &[]),
        &mut clock,
        summary,
    );
    wait(core, &mut clock, summary);
    wait(core, &mut clock, summary);
    // The peer answers the poll, so the token goes to it.
    let reply = Frame::new(FrameType::REPLY_TO_POLL_FOR_MASTER, STATION, PEER, &[]);
    receive(core, wire, reply, &mut clock, summary);
    wait(core, &mut clock, summary);
    // The peer uses the token: a broadcast shows it was taken up, then it
    // announces itself with a broadcast Test_Response, then a request that
    // this station answers at once.
    let broadcast = Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0xFF,
        PEER,
        &WHO_IS,
    );
    receive(core, wire, broadcast, &mut clock, summary);
    let announce = Frame::new(FrameType::TEST_RESPONSE, 0xFF, PEER, &[]);
    receive(core, wire, announce, &mut clock, summary);
    let request = Frame::new(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        STATION,
        PEER,
        &READ_PROPERTY,
    );
    receive(core, wire, request, &mut clock, summary);
    wait(core, &mut clock, summary);
    summary.state = core.state();
    fill_queues(core, oversize, clock.now, summary);
    record_counters(core, summary);
}

/// Fill the unicast queue until it refuses an NPDU, then offer `oversize`,
/// which no standard frame can carry.
fn fill_queues(core: &mut Core<'_>, oversize: &[u8], now: Instant, summary: &mut Summary) {
    for _ in 0..=UNICAST_SLOTS {
        match core.enqueue_npdu(PEER, &WHO_IS, NetworkPriority::Normal, now) {
            Ok(()) => summary.queued_before_full += 1,
            Err(QueueRefusal::Full) => break,
            Err(_) => summary.error(),
        }
    }
    if core.enqueue_npdu(PEER, oversize, NetworkPriority::Normal, now)
        != Err(QueueRefusal::Oversize)
    {
        summary.error();
    }
}

/// Take the core's counters, as firmware drains them, and copy the ones
/// the summary keeps: the scenario takes them once, so they are its totals.
fn record_counters(core: &mut Core<'_>, summary: &mut Summary) {
    let counters = core.take_counters();
    summary.frames_received = counters.frames_received;
    summary.frames_transmitted = counters.frames_transmitted;
    summary.token_rotations = counters.token_rotations;
    summary.parser_errors = counters.parser_errors;
    summary.drops_full = counters.unicast.drops_full;
    summary.drops_oversize = counters.unicast.drops_oversize;
    // Inputs the core ignored as broken: none in a good run.
    let broken = counters
        .adapter_errors
        .saturating_add(counters.internal_errors);
    summary.errors = summary
        .errors
        .saturating_add(u8::try_from(broken).unwrap_or(u8::MAX));
}

/// Feed `frame` to the core octet by octet, as the peer sends it, then
/// carry out what the core asks for.
fn receive(
    core: &mut Core<'_>,
    wire: &mut [u8],
    frame: Frame<'_>,
    clock: &mut Clock,
    summary: &mut Summary,
) {
    let Ok(length) = frame.encode_into(wire) else {
        summary.error();
        return;
    };
    for &octet in wire.iter().take(length) {
        clock.pass(1);
        core.on_octet(octet, clock.now);
    }
    drain(core, clock, summary);
}

/// Let time run to the core's next deadline, service it, and carry out what
/// the core asks for.
fn wait(core: &mut Core<'_>, clock: &mut Clock, summary: &mut Summary) {
    let Some(deadline) = core.next_deadline() else {
        summary.error();
        return;
    };
    clock.now = clock.now.max(deadline);
    core.on_deadline(clock.now);
    drain(core, clock, summary);
}

/// What one polled action leaves for the adapter to do once the action's
/// borrow of the core has ended.
enum Then {
    Nothing,
    Send { id: TransmitId },
    Answer(RequestId),
}

/// Poll actions until the core has none, carrying out each one: a transmit
/// starts at once and takes its octets' time on the line, and a request is
/// answered at once.
fn drain(core: &mut Core<'_>, clock: &mut Clock, summary: &mut Summary) {
    for _ in 0..MAX_ACTIONS {
        let then = match core.poll_action() {
            Action::Idle => return,
            Action::SetDirection(_) => {
                summary.direction_changes += 1;
                Then::Nothing
            }
            Action::Transmit { id, .. } => Then::Send { id },
            Action::DeliverNpdu { request_id, .. } => {
                summary.npdus_delivered += 1;
                match request_id {
                    Some(id) => Then::Answer(id),
                    None => Then::Nothing,
                }
            }
            Action::DeliverFrame { .. } => {
                summary.frames_delivered += 1;
                Then::Nothing
            }
        };
        match then {
            Then::Nothing => {}
            Then::Send { id } => {
                // The octets come with the confirmed start: send them from
                // the core's frame buffer.
                let octets = match core.on_transmit_started(id, clock.now) {
                    Ok(frame) => {
                        record_sent(frame, summary);
                        frame.len()
                    }
                    Err(_) => {
                        summary.error();
                        0
                    }
                };
                clock.pass(octets);
                core.on_transmit_complete(id, clock.now);
            }
            Then::Answer(id) => {
                summary.requests += 1;
                let answer = ReplyDecision::Data(&COMPLEX_ACK);
                if core.complete_request(id, answer, clock.now).is_err() {
                    summary.error();
                }
            }
        }
    }
    summary.error();
}

/// Note the type and destination of a frame the core sends.
fn record_sent(frame: &[u8], summary: &mut Summary) {
    let Ok((decoded, _)) = decode_frame(frame) else {
        summary.error();
        return;
    };
    match summary.sent.iter_mut().find(|slot| slot.is_none()) {
        Some(slot) => *slot = Some((decoded.header.frame_type, decoded.header.destination)),
        None => summary.error(),
    }
}
