//! The device's data link: the MS/TP core, driven the way a UART adapter
//! drives it, with the client's side of the line played from frames
//! written by hand.

use bacnet_mstp_codec::{decode_frame, Frame, FrameType};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Duration, Instant, MasterCore, ReplyDecision, RequestId, TransmitId,
};

use super::app::{self, Answer, Scratch};
use super::{
    read_answer, Core, Reply, Storage, Summary, CLIENT, CONFIG, GLOBAL_READ_PROPERTY, REQUESTS,
    STATION, WHO_IS_EVERY_DEVICE, WHO_IS_OTHERS,
};
use crate::tally::{fnv1a, FNV_OFFSET};

/// The MS/TP broadcast address.
const BROADCAST: u8 = 0xFF;

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

/// The application's buffers, beside the core's.
struct Application<'m> {
    scratch: &'m mut Scratch,
    answer: &'m mut [u8],
}

/// A frame from the client.
fn client(frame_type: FrameType, destination: u8, data: &[u8]) -> Frame<'_> {
    Frame::new(frame_type, destination, CLIENT, data)
}

/// Build the core over `storage` and run the script.
pub(super) fn run(
    storage: &mut Storage,
    scratch: &mut Scratch,
    answer: &mut [u8],
    wire: &mut [u8],
    summary: &mut Summary,
) {
    let mut clock = Clock {
        now: Instant::ZERO,
        octet: CONFIG.octet_time(),
    };
    let Ok(mut core) = MasterCore::new(CONFIG, storage, clock.now) else {
        summary.error();
        return;
    };
    let core = &mut core;
    let mut app = Application { scratch, answer };

    // The client looks for every device, which queues an I-Am, then for
    // devices 5000 to 6000, which this one isn't.
    for who_is in [&WHO_IS_EVERY_DEVICE[..], &WHO_IS_OTHERS] {
        let frame = client(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            BROADCAST,
            who_is,
        );
        receive(core, &mut app, wire, frame, &mut clock, summary);
    }
    // The client passes the token: the I-Am goes out, then, with no
    // successor known yet, a poll for the next station. The client answers
    // it, so the token goes back to it.
    let token = client(FrameType::TOKEN, STATION, &[]);
    receive(core, &mut app, wire, token, &mut clock, summary);
    wait(core, &mut app, &mut clock, summary);
    wait(core, &mut app, &mut clock, summary);
    let reply = client(FrameType::REPLY_TO_POLL_FOR_MASTER, STATION, &[]);
    receive(core, &mut app, wire, reply, &mut clock, summary);
    wait(core, &mut app, &mut clock, summary);
    // The client, holding the token, sends its requests one at a time, and
    // the device answers each in its reply window.
    for request in REQUESTS {
        let frame = client(FrameType::BACNET_DATA_EXPECTING_REPLY, STATION, request);
        receive(core, &mut app, wire, frame, &mut clock, summary);
        wait(core, &mut app, &mut clock, summary);
    }
    // Then two confirmed requests by broadcast, which the device leaves
    // unanswered: one in a broadcast frame, one sent to every network.
    for (destination, npdu) in [
        (BROADCAST, REQUESTS[0]),
        (STATION, &GLOBAL_READ_PROPERTY[..]),
    ] {
        let frame = client(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            destination,
            npdu,
        );
        receive(core, &mut app, wire, frame, &mut clock, summary);
    }
    summary.state = core.state();
    record_counters(core, summary);
}

/// Take the core's counters, as firmware drains them, and copy the ones
/// the summary keeps: the scenario takes them once, so they are its totals.
fn record_counters(core: &mut Core<'_>, summary: &mut Summary) {
    let counters = core.take_counters();
    summary.frames_received = counters.frames_received;
    summary.frames_transmitted = counters.frames_transmitted;
    summary.token_rotations = counters.token_rotations;
    summary.parser_errors = counters.parser_errors;
    // Inputs the core ignored as broken: none in a good run.
    let broken = counters
        .adapter_errors
        .saturating_add(counters.internal_errors);
    summary.errors = summary
        .errors
        .saturating_add(u8::try_from(broken).unwrap_or(u8::MAX));
}

/// Feed `frame` to the core octet by octet, as the client sends it, then
/// carry out what the core asks for.
fn receive(
    core: &mut Core<'_>,
    app: &mut Application<'_>,
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
    drain(core, app, clock, summary);
}

/// Let time run to the core's next deadline, service it, and carry out what
/// the core asks for.
fn wait(core: &mut Core<'_>, app: &mut Application<'_>, clock: &mut Clock, summary: &mut Summary) {
    let Some(deadline) = core.next_deadline() else {
        summary.error();
        return;
    };
    clock.now = clock.now.max(deadline);
    core.on_deadline(clock.now);
    drain(core, app, clock, summary);
}

/// What one polled action leaves for the adapter to do once the action's
/// borrow of the core has ended.
enum Then {
    Nothing,
    Send(TransmitId),
    /// Answer the request in its reply window with this many octets of
    /// the answer buffer, or abandon it with none.
    Reply(RequestId, Option<usize>),
    /// Queue this many octets of the answer buffer for the next token, to
    /// this station.
    Queue(u8, usize),
}

/// Poll actions until the core has none, carrying out each one: a transmit
/// starts at once and takes its octets' time on the line, a delivered NPDU
/// goes to the application, and its answer goes back to the core.
fn drain(core: &mut Core<'_>, app: &mut Application<'_>, clock: &mut Clock, summary: &mut Summary) {
    for _ in 0..MAX_ACTIONS {
        let then = match core.poll_action() {
            Action::Idle => return,
            Action::SetDirection(_) | Action::DeliverFrame { .. } => Then::Nothing,
            Action::Transmit { id, .. } => Then::Send(id),
            Action::DeliverNpdu {
                source,
                destination,
                npdu,
                request_id,
            } => {
                summary.delivered = summary.delivered.saturating_add(1);
                let answer = app::answer(npdu, app.scratch, app.answer);
                // A confirmed request in a broadcast frame gets no answer:
                // only unconfirmed services travel by broadcast (Clause
                // 5.4.5.1). The application can't see the frame's
                // destination, so the link drops its reply here.
                let answer = match answer {
                    Answer::Reply(_) if destination == BROADCAST => Answer::Nothing,
                    answer => answer,
                };
                route(answer, source, request_id, summary)
            }
        };
        match then {
            Then::Nothing => {}
            Then::Send(id) => {
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
            Then::Reply(id, length) => {
                let decision = match length.and_then(|length| app.answer.get(..length)) {
                    Some(npdu) => ReplyDecision::Data(npdu),
                    None => ReplyDecision::Abandoned,
                };
                if core.complete_request(id, decision, clock.now).is_err() {
                    summary.error();
                }
            }
            Then::Queue(destination, length) => {
                let npdu = app.answer.get(..length).unwrap_or_default();
                if core
                    .enqueue_npdu(destination, npdu, NetworkPriority::Normal, clock.now)
                    .is_err()
                {
                    summary.error();
                }
            }
        }
    }
    summary.error();
}

/// Where the application's answer goes. A reply to a data-expecting-reply
/// frame goes in the core's reply window; an I-Am, and a reply to a request
/// that came without one (which a peer shouldn't send), wait in the queue
/// for the token; a request with no answer is abandoned, so the core stops
/// waiting for one.
fn route(answer: Answer, source: u8, request_id: Option<RequestId>, summary: &mut Summary) -> Then {
    match (answer, request_id) {
        (Answer::Reply(length), Some(id)) => Then::Reply(id, Some(length)),
        (Answer::Reply(length), None) => Then::Queue(source, length),
        (Answer::Broadcast(length), None) => Then::Queue(BROADCAST, length),
        // An I-Am answering a data-expecting-reply frame: the frame gets no
        // reply, and the I-Am waits for the token.
        (Answer::Broadcast(length), Some(_)) => {
            summary.error();
            Then::Queue(BROADCAST, length)
        }
        (Answer::Nothing, request) => {
            summary.unanswered = summary.unanswered.saturating_add(1);
            match request {
                Some(id) => Then::Reply(id, None),
                None => Then::Nothing,
            }
        }
    }
}

/// Note a frame the device sends, as the client reads it.
fn record_sent(frame: &[u8], summary: &mut Summary) {
    let Ok((decoded, _)) = decode_frame(frame) else {
        summary.error();
        return;
    };
    let header = decoded.header;
    let recorded = if decoded.data.is_empty() {
        let slot = summary.link_frames.iter_mut().find(|slot| slot.is_none());
        slot.map(|slot| *slot = Some((header.frame_type, header.destination)))
    } else {
        let reply = Reply {
            frame_type: header.frame_type,
            destination: header.destination,
            pdu: read_answer(decoded.data),
            length: u8::try_from(decoded.data.len()).unwrap_or(u8::MAX),
            fnv: fnv1a(FNV_OFFSET, decoded.data),
        };
        let slot = summary.replies.iter_mut().find(|slot| slot.is_none());
        slot.map(|slot| *slot = Some(reply))
    };
    if recorded.is_none() {
        summary.error();
    }
}
