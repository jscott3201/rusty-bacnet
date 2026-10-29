//! The firmware `device` image's application on a simulated bus (#1796).
//!
//! The image's own scenario (`firmware/link`) drives its core from a
//! script, with one client. Here the same `answer()` runs as a simulated
//! master's application, on a ring with two other masters, so the token
//! passing, the reply window and the queued I-Am all come from real cores
//! on one line.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::{
    Answer, Application, BROADCAST, MasterSpec, Outbox, Received, Sim, SimConfig, WireFrame,
};
use firmware_link::device::app::{self, ANSWER_LENGTH, Scratch};
use firmware_link::device::{
    BROADCAST_SLOTS, CONFIG, REQUESTS, STATION, UNICAST_SLOTS, WHO_IS_EVERY_DEVICE,
};

use common::*;

/// The other two masters: the client that reads a property, and one that
/// asks who is there.
const CLIENT: u8 = 0;
const ASKER: u8 = 2;
/// How long the device takes to work out an answer.
const THINKING: Duration = Duration::from_millis(3);
/// Clause 9.5.3's Treply_delay: the longest a node may take to start
/// answering a data-expecting-reply frame.
const T_REPLY_DELAY: Duration = Duration::from_millis(250);

/// The image's application with its buffers, as a simulated master's
/// application. A reply to a request in a broadcast frame is dropped, as
/// the image's link does, since `answer()` can't see the frame's
/// destination.
struct Device {
    scratch: Scratch,
    out: [u8; ANSWER_LENGTH],
}

impl Application for Device {
    fn on_npdu(&mut self, received: &Received<'_>, outbox: &mut Outbox) -> Option<Answer> {
        match app::answer(received.npdu, &mut self.scratch, &mut self.out) {
            app::Answer::Reply(length) if received.destination != BROADCAST => {
                Some(Answer::Reply {
                    delay: THINKING,
                    npdu: self.out[..length].to_vec(),
                })
            }
            app::Answer::Broadcast(length) => {
                outbox.broadcast(&self.out[..length]);
                None
            }
            app::Answer::Reply(_) | app::Answer::Nothing => None,
        }
    }
}

/// What the application answers `npdu` with, run on its own.
fn answer_to(npdu: &[u8]) -> Vec<u8> {
    let mut out = [0; ANSWER_LENGTH];
    let length = match app::answer(npdu, &mut Scratch::new(), &mut out) {
        app::Answer::Reply(length) | app::Answer::Broadcast(length) => length,
        app::Answer::Nothing => panic!("no answer to {npdu:02X?}"),
    };
    out[..length].to_vec()
}

/// A ring of three masters at the device's baud rate, the device in the
/// middle with the image's configuration and queue sizes.
fn ring_with_device() -> Sim {
    let baud = CONFIG.baud_rate;
    let mut sim = Sim::new(SimConfig::new(baud));
    sim.add_master(MasterSpec::new(CLIENT, CONFIG.max_master, baud));
    let mut device = MasterSpec::new(STATION, CONFIG.max_master, baud)
        .queues(UNICAST_SLOTS, BROADCAST_SLOTS)
        .application(Device {
            scratch: Scratch::new(),
            out: [0; ANSWER_LENGTH],
        });
    device.config = CONFIG;
    sim.add_master(device);
    sim.add_master(MasterSpec::new(ASKER, CONFIG.max_master, baud));
    sim
}

fn position(frames: &[WireFrame], predicate: impl Fn(&WireFrame) -> bool) -> usize {
    frames
        .iter()
        .position(|frame| frame.is_clean() && predicate(frame))
        .expect("the frame went out")
}

/// Clauses 9.5.6.2 and 9.5.6.9, with Clause 15.5: the device answers a
/// ReadProperty of its Object_Name in the reply window, as the next frame
/// after the request and inside Treply_delay, with the ComplexACK its
/// application wrote.
#[test]
fn the_device_answers_a_read_property_in_the_reply_window() {
    let mut sim = ring_with_device();
    let request = REQUESTS[0];
    sim.enqueue(ms(1_000), CLIENT, STATION, request);
    sim.run_until(ms(3_000));
    sim.assert_no_violations();

    let frames = sim.frames();
    let asked = position(frames, |frame| {
        frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, CLIENT, STATION)
    });
    let reply = &frames[asked + 1];
    assert!(
        reply.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, STATION, CLIENT),
        "the reply is the next frame: {reply:?}"
    );
    let waited = reply.start - frames[asked].end;
    assert!(
        waited <= T_REPLY_DELAY,
        "the reply started after {waited:?}"
    );
    assert!(waited >= THINKING, "the reply came before the answer did");
    assert_eq!(reply.data, answer_to(request));
    // A ComplexACK (0x30) for invoke ID 1, of a ReadProperty (0x0C).
    assert_eq!(reply.data[2..5], [0x30, 0x01, 0x0C]);

    assert!(
        sim.deliveries(STATION)
            .iter()
            .any(|delivery| delivery.request && delivery.data == request)
    );
    assert!(
        sim.deliveries(CLIENT)
            .iter()
            .any(|delivery| delivery.source == STATION && delivery.data == reply.data)
    );
}

/// Clause 16.10: a Who-Is for every device, broadcast by a master, gets an
/// I-Am that the device queues as a broadcast and sends when it next holds
/// the token; both other masters receive it.
#[test]
fn the_device_answers_who_is_with_an_i_am() {
    let mut sim = ring_with_device();
    sim.enqueue(ms(1_000), ASKER, BROADCAST, &WHO_IS_EVERY_DEVICE);
    sim.run_until(ms(3_000));
    sim.assert_no_violations();

    let frames = sim.frames();
    let asked = position(frames, |frame| {
        frame.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, ASKER, BROADCAST)
    });
    let i_am = answer_to(&WHO_IS_EVERY_DEVICE);
    // An unconfirmed request (0x10) of I-Am (0x00), after the NPCI with
    // DNET 0xFFFF, an empty DADR and the hop count.
    assert_eq!(i_am[6..8], [0x10, 0x00], "{i_am:02X?}");
    let sent = position(frames, |frame| {
        frame.is(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            STATION,
            BROADCAST,
        )
    });
    assert!(sent > asked);
    assert_eq!(frames[sent].data, i_am);
    // The device sends it in its own token hold: the frame before it hands
    // the device the token.
    assert!(frames[sent - 1].is(FrameType::TOKEN, CLIENT, STATION));
    for station in [CLIENT, ASKER] {
        assert!(
            sim.deliveries(station)
                .iter()
                .any(|delivery| delivery.source == STATION && delivery.data == i_am),
            "station {station} got no I-Am"
        );
    }
}
