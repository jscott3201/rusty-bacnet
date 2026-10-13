//! Helpers shared by the scenario files. Each test binary uses a subset.

#![allow(dead_code)]

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{MasterSpec, Sim, SimConfig, WireFrame};

/// One of the two baud rates every MS/TP device supports (Clause 9.2.3).
pub const BAUD: u32 = 38_400;

/// The other required rate.
pub const SLOW_BAUD: u32 = 9_600;

pub fn ms(millis: u64) -> Instant {
    Instant::from_micros(millis * 1_000)
}

pub fn us(micros: u64) -> Instant {
    Instant::from_micros(micros)
}

pub fn millis(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A strict simulator with one standard master per station.
pub fn ring(baud: u32, stations: &[u8], max_master: u8) -> Sim {
    let mut sim = Sim::new(SimConfig::new(baud));
    for &station in stations {
        sim.add_master(MasterSpec::new(station, max_master, baud));
    }
    sim
}

/// Clean frames of one type from `source`.
pub fn sent_by(frames: &[WireFrame], source: u8, frame_type: FrameType) -> Vec<&WireFrame> {
    frames
        .iter()
        .filter(|frame| frame.is_clean())
        .filter(|frame| frame.source() == Some(source) && frame.frame_type() == Some(frame_type))
        .collect()
}

/// The first clean frame matching `predicate` that starts at or after `since`.
pub fn first_after(
    frames: &[WireFrame],
    since: Instant,
    predicate: impl Fn(&WireFrame) -> bool,
) -> Option<&WireFrame> {
    frames
        .iter()
        .find(|frame| frame.start >= since && frame.is_clean() && predicate(frame))
}

/// Run until a clean frame matching `predicate` has ended at or after
/// `since`, in steps of one bit time at most, and return when it ended.
///
/// # Panics
///
/// If none has by `limit`.
pub fn run_until_frame(
    sim: &mut Sim,
    since: Instant,
    limit: Instant,
    predicate: impl Fn(&WireFrame) -> bool,
) -> Instant {
    sim.run_until(since);
    loop {
        if let Some(frame) = sim
            .frames()
            .iter()
            .rev()
            .find(|frame| frame.end >= since && frame.is_clean() && predicate(frame))
        {
            return frame.end;
        }
        assert!(sim.now() < limit, "no matching frame by {limit:?}");
        let step = sim.timing().bits(1);
        sim.run_for(step);
    }
}

/// A data NPDU (network version 1); `expecting_reply` sets the control
/// octet's data-expecting-reply bit.
pub fn npdu(expecting_reply: bool, tag: u8) -> Vec<u8> {
    vec![0x01, if expecting_reply { 0x04 } else { 0x00 }, tag]
}

/// Render the wire log for an assertion message.
pub fn log(frames: &[WireFrame]) -> String {
    frames.iter().map(|frame| format!("  {frame}\n")).collect()
}
