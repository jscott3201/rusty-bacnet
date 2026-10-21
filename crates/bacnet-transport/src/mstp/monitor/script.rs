//! Test support: a scripted MS/TP line and a token ring that plays on it,
//! public with the `test-support` feature so that tools built on the
//! monitor can be tested without hardware.
//!
//! [`LineScript`] lays frames, corrupted frames, noise and silences out on
//! a timeline at a baud rate, each as the chunk a read would return when
//! its last octet arrived. [`ScriptedRing`] drives a ring of masters over
//! one, passing the token in station order and polling for masters the
//! way Clause 9.5.6 has a holder do (one maintenance poll per hold here,
//! rather than one every Npoll tokens), so a test can set up a misconfigured
//! ring, inject errors or lose the token and know exactly what a monitor
//! should report.

use std::time::Duration;

use bacnet_mstp_codec::{Frame, FrameType, MAX_COBS_FRAME_LENGTH};

use super::stats::octet_time;

/// Gap between one frame and the next station's answer or frame.
pub const TURNAROUND: Duration = Duration::from_millis(1);
/// How long a poller waits for an answer that doesn't come: T_usage_timeout.
pub const USAGE_TIMEOUT: Duration = Duration::from_millis(20);

/// What to damage in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// Flip bits of the header CRC octet.
    HeaderCrc,
    /// Flip bits of the last data CRC octet (the frame must carry data).
    DataCrc,
}

/// Octets laid out in time: each chunk with the time its last octet
/// arrived.
#[derive(Debug, Clone)]
pub struct LineScript {
    baud_rate: u32,
    now: Duration,
    chunks: Vec<(Duration, Vec<u8>)>,
}

impl LineScript {
    /// An empty line at `baud_rate`, at time zero.
    #[must_use]
    pub fn new(baud_rate: u32) -> Self {
        Self {
            baud_rate,
            now: Duration::ZERO,
            chunks: Vec::new(),
        }
    }

    /// The chunks so far, each with the time its last octet arrived.
    #[must_use]
    pub fn chunks(&self) -> &[(Duration, Vec<u8>)] {
        &self.chunks
    }

    /// Let the line stay silent for `silence`.
    pub fn idle(&mut self, silence: Duration) -> &mut Self {
        self.now += silence;
        self
    }

    /// Put `octets` on the line now, as one chunk.
    pub fn raw(&mut self, octets: &[u8]) -> &mut Self {
        let octet = octet_time(self.baud_rate);
        self.now += octet * u32::try_from(octets.len()).unwrap();
        self.chunks.push((self.now, octets.to_vec()));
        self
    }

    /// Send a frame now, then wait [`TURNAROUND`].
    pub fn frame(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        source: u8,
        data: &[u8],
    ) -> &mut Self {
        let octets = encode(frame_type, destination, source, data);
        self.raw(&octets).idle(TURNAROUND)
    }

    /// Send a frame with `damage` done to it, then wait [`TURNAROUND`].
    pub fn damaged(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        source: u8,
        data: &[u8],
        damage: Damage,
    ) -> &mut Self {
        let mut octets = encode(frame_type, destination, source, data);
        let index = match damage {
            Damage::HeaderCrc => 7,
            Damage::DataCrc => {
                assert!(!data.is_empty(), "a data CRC needs data");
                octets.len() - 1
            }
        };
        // Avoid 0x55, which would look like the start of a preamble.
        octets[index] ^= 0x0F;
        self.raw(&octets).idle(TURNAROUND)
    }

    /// The same octets regrouped as a coarse adapter would hand them over:
    /// everything that arrived in each `window` comes in one read at the
    /// window's end.
    pub fn rechunked(&self, window: Duration) -> Vec<(Duration, Vec<u8>)> {
        let octet = octet_time(self.baud_rate);
        let mut out: Vec<(Duration, Vec<u8>)> = Vec::new();
        for (end, octets) in &self.chunks {
            let count = u32::try_from(octets.len()).unwrap();
            let first = *end - octet * count;
            for (index, value) in octets.iter().enumerate() {
                let arrived = first + octet * (u32::try_from(index).unwrap() + 1);
                let slot = arrived.as_nanos().div_ceil(window.as_nanos());
                let read_at = window * u32::try_from(slot).unwrap();
                match out.last_mut() {
                    Some((at, chunk)) if *at == read_at => chunk.push(*value),
                    _ => out.push((read_at, vec![*value])),
                }
            }
        }
        out
    }
}

fn encode(frame_type: FrameType, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let length = Frame::new(frame_type, destination, source, data)
        .encode_into(&mut wire)
        .unwrap();
    wire.truncate(length);
    wire
}

/// Run timed chunks through a decoder and statistics engine, as the
/// monitor's task does.
#[cfg(test)]
pub(crate) fn analyse(
    baud_rate: u32,
    chunks: &[(Duration, Vec<u8>)],
) -> super::stats::MonitorStats {
    use super::line::{feed_chunk, LineClock};
    use bacnet_mstp_codec::{DecodeEvent, StreamDecoder};
    let mut stats = super::stats::MonitorStats::new(baud_rate, Duration::ZERO);
    let mut storage = vec![0u8; MAX_COBS_FRAME_LENGTH + 512];
    let mut decoder = StreamDecoder::new(&mut storage).with_reserved_cobs();
    let mut clock = LineClock::new(baud_rate, Duration::from_millis(100));
    for (at, chunk) in chunks {
        stats.record_read(chunk.len());
        feed_chunk(&mut decoder, &mut clock, chunk, *at, |at, event, _| {
            stats.record(at, &event);
        });
    }
    assert!(matches!(decoder.push_slice(&[]), DecodeEvent::NeedMore));
    stats
}

#[derive(Debug, Clone)]
struct RingMaster {
    station: u8,
    max_master: u8,
    /// PS: the station last polled, or the master itself.
    poll_station: u8,
}

/// A token ring of masters on a [`LineScript`].
#[derive(Debug, Clone)]
pub struct ScriptedRing {
    /// The line the ring plays on.
    pub line: LineScript,
    masters: Vec<RingMaster>,
    holder: usize,
}

impl ScriptedRing {
    /// A ring with no masters yet, on an empty line at `baud_rate`.
    #[must_use]
    pub fn new(baud_rate: u32) -> Self {
        Self {
            line: LineScript::new(baud_rate),
            masters: Vec::new(),
            holder: 0,
        }
    }

    /// Add a master in the ring. Masters pass the token in station order.
    pub fn master(mut self, station: u8, max_master: u8) -> Self {
        self.masters.push(RingMaster {
            station,
            max_master,
            poll_station: station,
        });
        self.masters.sort_by_key(|master| master.station);
        self
    }

    fn next_station(&self, index: usize) -> u8 {
        self.masters[(index + 1) % self.masters.len()].station
    }

    /// Let `holds` token holds go by, starting with the current holder.
    /// `traffic` runs at the start of each, with the holder's station, to
    /// send its data frames.
    pub fn rotate(
        &mut self,
        holds: usize,
        mut traffic: impl FnMut(&mut LineScript, u8),
    ) -> &mut Self {
        for _ in 0..holds {
            let index = self.holder;
            let station = self.masters[index].station;
            traffic(&mut self.line, station);
            let next = self.next_station(index);
            let master = &mut self.masters[index];
            let modulus = u16::from(master.max_master) + 1;
            let candidate = ((u16::from(master.poll_station) + 1) % modulus) as u8;
            if candidate == next || candidate == station {
                // ResetMaintenancePFM: start the next cycle from the top.
                master.poll_station = station;
            } else {
                // SendMaintenancePFM: nobody answers.
                master.poll_station = candidate;
                self.line
                    .frame(FrameType::POLL_FOR_MASTER, candidate, station, &[])
                    .idle(USAGE_TIMEOUT - TURNAROUND);
            }
            self.line.frame(FrameType::TOKEN, next, station, &[]);
            self.holder = (index + 1) % self.masters.len();
        }
        self
    }

    /// Lose the token: the line goes silent for `silence`, then the lowest
    /// master generates a new one, polls its successor, hears it answer and
    /// passes it the token (Clause 9.5.6.7, 9.5.6.8).
    pub fn lose_token(&mut self, silence: Duration) -> &mut Self {
        self.line.idle(silence);
        let lowest = self.masters[0].station;
        let next = self.next_station(0);
        self.line
            .frame(FrameType::POLL_FOR_MASTER, next, lowest, &[])
            .frame(FrameType::REPLY_TO_POLL_FOR_MASTER, lowest, next, &[])
            .frame(FrameType::TOKEN, next, lowest, &[]);
        self.holder = 1 % self.masters.len();
        self
    }
}
