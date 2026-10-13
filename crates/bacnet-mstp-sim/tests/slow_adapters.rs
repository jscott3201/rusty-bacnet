//! Adapters that take a while to start a frame (#1625).
//!
//! From the moment the core hands an adapter a frame (`Action::Transmit`)
//! until the adapter confirms the start (`on_transmit_started`), what the
//! receiver hears is line activity, not echo: echo can only begin once the
//! driver is on. Clause 9.5.5.1 has the sender wait for Tturnaround of
//! silence right before enabling its driver, so the core refuses a start
//! that comes within Tturnaround of that activity, and the frame waits for
//! a quiet line again.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{AdapterSpec, Fault, MasterSpec, ScriptSend, Sim, SimConfig};

use common::*;

/// A ring of 0 and 3, run until station 3 has just received a token, from
/// when station 3's adapter starts frames `delay` after its core hands them
/// over. Returns when that token ended.
fn slow_station_3(config: SimConfig, delay: Duration) -> (Sim, Instant) {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 3, BAUD))
        .add_master(MasterSpec::new(3, 3, BAUD));
    let token_end = run_until_frame(&mut sim, ms(1_500), ms(3_000), |frame| {
        frame.is(FrameType::TOKEN, 0, 3)
    });
    let now = sim.now();
    sim.inject(
        now,
        Fault::SetAdapter {
            station: 3,
            adapter: AdapterSpec {
                start_delay: delay,
                ..AdapterSpec::default()
            },
        },
    );
    (sim, token_end)
}

/// Station 3's core hands its next frame over Tturnaround after the token,
/// and its adapter would start it 4 ms later. A noise octet that ends
/// 0.28 ms before that start is activity station 3 must wait Tturnaround
/// after: the core refuses the start, and the frame goes out Tturnaround
/// after the noise, plus the adapter's delay.
#[test]
fn a_slow_adapter_never_starts_within_turnaround_of_activity() {
    let (mut sim, token_end) = slow_station_3(SimConfig::new(BAUD), Duration::from_millis(4));
    let noise_at = token_end + Duration::from_micros(4_500);
    sim.inject(noise_at, Fault::Noise { octets: vec![0x00] });
    sim.run_until(token_end + millis(100));
    sim.assert_no_violations();
    let noise_end = noise_at + sim.timing().octet();
    let next = first_after(sim.frames(), token_end, |frame| frame.source() == Some(3))
        .expect("station 3's frame");
    assert!(next.is(FrameType::TOKEN, 3, 0), "{next}");
    assert_eq!(
        next.start,
        noise_end + sim.timing().bits(40) + Duration::from_millis(4)
    );
    assert_eq!(sim.report().refused_starts, 1);
    let counters = sim.core(3).unwrap().counters();
    assert_eq!(counters.deferred_starts, 1);
    assert_eq!(counters.cancelled_sends, 0);
}

/// With a start delay longer than Tusage_timeout, station 0 sends the token
/// a second time before station 3's first frame is out. Station 3's core,
/// waiting for its adapter, hears that Token whole and counts it. It already
/// holds the token, so it drops the copy rather than take it twice. The
/// retry ends more than Tturnaround before the adapter's start, so the start
/// goes ahead, and station 0 takes station 3's frame as its successor using
/// the token. Every round of the ring goes the same way.
#[test]
fn a_frame_that_arrives_before_a_slow_start_is_still_heard() {
    let (mut sim, token_end) = slow_station_3(SimConfig::new(BAUD), Duration::from_millis(25));
    sim.run_until(token_end + millis(100));
    sim.assert_no_violations();
    let frames = sim.frames();
    let retry = first_after(frames, token_end, |_| true).expect("the retry");
    assert!(retry.is(FrameType::TOKEN, 0, 3), "{retry}");
    assert_eq!(retry.start, token_end + millis(20));
    let pass = first_after(frames, retry.end, |_| true).expect("station 3's pass");
    assert!(pass.is(FrameType::TOKEN, 3, 0), "{pass}");
    assert_eq!(
        pass.start,
        token_end + sim.timing().bits(40) + Duration::from_millis(25)
    );
    // One dropped retry for each of station 3's passes.
    let passes = sent_by(frames, 3, FrameType::TOKEN)
        .into_iter()
        .filter(|frame| frame.start > token_end)
        .count();
    let counters = sim.core(3).unwrap().counters();
    assert_eq!(passes, 3);
    assert_eq!(counters.discarded_while_busy, 3);
    assert_eq!(counters.cancelled_sends, 0);
    assert_eq!(sim.report().refused_starts, 0);
}

/// A deferred start leaves the adapter expecting the same frame back. If
/// the core gives that frame up instead, a later frame with the same octets
/// is a new one. Here a lone master's generating poll is deferred by a noise
/// octet, then dropped as another station's Token arrives (SawFrame). The
/// poll station 0 then sends as token holder (NextStationUnknown) is the
/// same Poll For Master 0->1, and must not pass for a generating one, which
/// would excuse a collision as a generation race.
#[test]
fn a_dropped_deferred_poll_does_not_mark_the_next_one_as_generating() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    let slow = AdapterSpec {
        start_delay: Duration::from_millis(3),
        ..AdapterSpec::default()
    };
    sim.add_master(MasterSpec::new(0, 3, BAUD).adapter(slow));
    // Station 0 generates at 500 ms; its adapter would start at 503 ms.
    sim.inject(us(502_500), Fault::Noise { octets: vec![0x00] });
    let token = ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 2, &[]).octets;
    sim.inject(us(503_100), Fault::Noise { octets: token });
    sim.run_until(ms(600));
    let counters = sim.core(0).unwrap().counters();
    assert_eq!(counters.deferred_starts, 1);
    assert_eq!(counters.cancelled_sends, 1);
    let poll = first_after(sim.frames(), us(503_100), |frame| {
        frame.is(FrameType::POLL_FOR_MASTER, 0, 1)
    })
    .expect("station 0's poll as token holder");
    assert!(!poll.generation, "{poll}");
    sim.assert_no_violations();
}
