//! Two MS/TP master nodes on a simulated line, each driven the way a UART
//! adapter drives `bacnet-mstp-core` (#1771).
//!
//! ```text
//! cargo run -p bacnet-mstp-core --example node
//! ```
//!
//! Nothing here reads a wall clock or a serial port. Time is a virtual
//! clock the example advances itself: by one octet time for each octet on
//! the line, and to the nearest deadline when the line is quiet. On
//! firmware the same `Instant` comes from a free-running timer, for
//! example `Instant::from_micros(ticks)`, and the octets from a UART. The
//! line is ideal: what one station sends, the other hears octet by octet,
//! and only one station sends at a time, as the token protocol arranges.
//!
//! Station 0 queues a Who-Is broadcast and a request for station 1, which
//! station 1 answers in its reply window. The run prints every frame, every
//! delivery, and each station's counters at the end.
//!
//! The adapter follows the core's three rules (see the crate docs, "The
//! adapter's side"): poll after every call; take a frame's octets from
//! `on_transmit_started`, so the line is driven only for a confirmed start;
//! and report the end of the frame with `on_transmit_complete`.

#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use bacnet_mstp_core::codec::decode_frame;
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Duration, FixedQueues, Instant, MasterCore, MstpCoreConfig, ReplyDecision,
    StandardStorage, TransmitId,
};
use bacnet_time::earliest;

/// Two unicast and two broadcast queue slots, and one delivery slot: the
/// adapter polls after every call. On firmware this lives in a static.
type Storage = StandardStorage<2, 2, 1>;
/// A core over borrowed storage.
type Core<'s> = MasterCore<'s, &'s mut FixedQueues<2, 2>>;

/// A Who-Is for every device: a local NPDU, no reply expected.
const WHO_IS: [u8; 4] = [0x01, 0x00, 0x10, 0x08];
/// A ReadProperty of Device 1's Object_Name: the control octet (0x04)
/// asks for a reply, so it goes out as a data-expecting-reply frame.
const READ_PROPERTY: [u8; 13] = [
    0x01, 0x04, 0x00, 0x03, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// Station 1's answer, a ComplexACK with the name "node".
const ANSWER: [u8; 20] = [
    0x01, 0x00, 0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D, 0x3E, 0x75, 0x05, 0x00,
    b'n', b'o', b'd', b'e',
];

/// How long the example runs, in virtual time.
const RUN_FOR: Duration = Duration::from_millis(560);

/// The virtual clock: the only source of `now`.
struct Clock {
    now: Instant,
    octet: Duration,
}

/// One station: its core, and a transmit it was asked for and hasn't
/// started yet.
struct Station<'s> {
    address: u8,
    core: Core<'s>,
    pending: Option<TransmitId>,
}

impl Station<'_> {
    /// Poll actions until the core is idle, carrying out each but a
    /// transmit, which waits for the line (`pending`).
    fn drain(&mut self, clock: &Clock) {
        loop {
            let reply = match self.core.poll_action() {
                Action::Idle => return,
                Action::SetDirection(_) => None,
                Action::Transmit { id, .. } => {
                    self.pending = Some(id);
                    None
                }
                Action::DeliverNpdu {
                    source,
                    npdu,
                    request_id,
                    ..
                } => {
                    println!(
                        "{:>9} station {} delivers {} octets from station {source}",
                        millis(clock.now),
                        self.address,
                        npdu.len()
                    );
                    request_id
                }
                Action::DeliverFrame { frame_type, .. } => {
                    println!(
                        "{:>9} station {} delivers {frame_type:?}",
                        millis(clock.now),
                        self.address
                    );
                    None
                }
            };
            // A request expecting a reply: answer it in the reply window,
            // once the action's borrow of the core has ended.
            if let Some(id) = reply {
                let decision = ReplyDecision::Data(&ANSWER);
                if let Err(error) = self.core.complete_request(id, decision, clock.now) {
                    println!(
                        "station {}: the answer was refused: {error:?}",
                        self.address
                    );
                }
            }
        }
    }
}

/// Send `station`'s pending frame: confirm the start, put each octet on
/// the line, where `listener` hears it one octet time later, then report
/// the end.
fn send(station: &mut Station<'_>, listener: &mut Station<'_>, clock: &mut Clock) {
    let Some(id) = station.pending.take() else {
        return;
    };
    // The octets come with the confirmed start, so the driver goes on only
    // for a frame the core still wants sent. A refused start comes back
    // later as a new Transmit.
    let mut wire = [0u8; 512];
    let length = match station.core.on_transmit_started(id, clock.now) {
        Ok(frame) => {
            if let Ok((decoded, _)) = decode_frame(frame) {
                let header = decoded.header;
                println!(
                    "{:>9} station {} -> {:<3} {:?} ({} octets)",
                    millis(clock.now),
                    header.source,
                    header.destination,
                    header.frame_type,
                    frame.len()
                );
            }
            wire[..frame.len()].copy_from_slice(frame);
            frame.len()
        }
        Err(error) => {
            println!("station {}: start refused: {error:?}", station.address);
            return;
        }
    };
    for &octet in &wire[..length] {
        clock.now += clock.octet;
        listener.core.on_octet(octet, clock.now);
        listener.drain(clock);
    }
    station.core.on_transmit_complete(id, clock.now);
    station.drain(clock);
}

/// `at` as milliseconds since the start, for the log.
fn millis(at: Instant) -> String {
    let micros = at.as_nanos() / 1_000;
    format!("{}.{:03} ms", micros / 1_000, micros % 1_000)
}

/// Station `address` of masters 0 and 1, at 38400 baud.
fn config(address: u8) -> MstpCoreConfig {
    let mut config = MstpCoreConfig::for_baud_rate(38_400).expect("a standard baud rate");
    config.station = address;
    config.max_master = 1;
    config
}

fn main() {
    let mut clock = Clock {
        now: Instant::ZERO,
        octet: config(0).octet_time(),
    };
    let (mut storage_0, mut storage_1) = (Storage::new(), Storage::new());
    let mut stations = [(0, &mut storage_0), (1, &mut storage_1)].map(|(address, storage)| {
        let core = MasterCore::new(config(address), storage, clock.now).expect("a valid config");
        Station {
            address,
            core,
            pending: None,
        }
    });
    let [zero, one] = &mut stations;
    for (destination, npdu) in [(0xFF, &WHO_IS[..]), (1, &READ_PROPERTY)] {
        zero.core
            .enqueue_npdu(destination, npdu, NetworkPriority::Normal, clock.now)
            .expect("room in the queue");
    }

    let end = Instant::ZERO + RUN_FOR;
    while clock.now < end {
        // A station with a frame to send drives the line; the other hears
        // it. Otherwise time jumps to the next deadline either core has:
        // `earliest` skips a core with none, where `Option::min` would
        // pick `None` and the timers would never fire.
        if zero.pending.is_some() {
            send(zero, one, &mut clock);
        } else if one.pending.is_some() {
            send(one, zero, &mut clock);
        } else {
            let Some(next) = earliest([zero.core.next_deadline(), one.core.next_deadline()]) else {
                break;
            };
            clock.now = clock.now.max(next);
            for station in [&mut *zero, &mut *one] {
                if station
                    .core
                    .next_deadline()
                    .is_some_and(|at| at <= clock.now)
                {
                    station.core.on_deadline(clock.now);
                    station.drain(&clock);
                }
            }
        }
    }

    for station in [zero, one] {
        let counters = station.core.take_counters();
        println!(
            "station {}: {:?}, {} frames received, {} transmitted, {} token passes",
            station.address,
            station.core.state(),
            counters.frames_received,
            counters.frames_transmitted,
            counters.token_rotations
        );
    }
}
