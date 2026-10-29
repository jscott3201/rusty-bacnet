//! What the checker knows of the station, worked out from the configuration
//! and the standard rather than asked of the core, and the frames on the
//! line.

use bacnet_mstp_codec::crc::crc8;
use bacnet_mstp_codec::{
    BROADCAST_MAC, Frame, FrameType, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA,
    decode_frame_in_place,
};
use bacnet_mstp_core::{Duration, MstpCoreConfig};

use crate::input::{Address, FrameSpec, Kind, fill};

/// Npoll: token uses between maintenance polls (Clause 9.5.3).
pub const N_POLL: u64 = 50;
/// Nmin_octets: more octets than this after a Token shows its receiver used
/// it (Clauses 9.5.3, 9.5.6.6).
pub const N_MIN_OCTETS: u64 = 4;
/// The shortest T_reply_timeout any station may use (Clause 9.5.3): an
/// answer to a data request or Test_Request must have its first octet heard
/// before it.
pub const MIN_REPLY_TIMEOUT: u64 = 255_000;
/// The shortest T_usage_timeout any station may use (Clause 9.5.3): the same
/// for a Reply To Poll For Master.
pub const MIN_USAGE_TIMEOUT: u64 = 20_000;
/// T_slot (Clause 9.5.3).
pub const T_SLOT: u64 = 10_000;
/// T_no_token (Clause 9.5.3).
pub const T_NO_TOKEN: u64 = 500_000;

/// The station's parameters, in µs where they are times.
///
/// The core counts nanoseconds, but the target works in whole microseconds:
/// every timer the setup gives the core is one, and so is every step's time,
/// which the driver hands over with `Instant::from_micros`. So the core's
/// deadlines are whole microseconds too, and this model of them is exact.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// TS.
    pub station: u8,
    /// Max_Master.
    pub max_master: u8,
    /// The data frames one token use may carry: Max_Info_Frames, scaled
    /// down by the core's local `multiple_frame_ratio` (rounded up, at
    /// least 1).
    pub frame_limit: u64,
    /// Max_Info_Frames itself.
    pub max_info_frames: u64,
    /// One octet on the wire: ten bit times, rounded up.
    pub octet_time: u64,
    /// T_turnaround.
    pub turnaround: u64,
    /// T_frame_abort.
    pub frame_abort: u64,
    /// T_reply_timeout.
    pub reply_timeout: u64,
    /// T_usage_timeout.
    pub usage_timeout: u64,
    /// T_reply_delay.
    pub reply_delay: u64,
    /// T_usage_delay.
    pub usage_delay: u64,
    /// The core's `reply_transmit_margin`: it sends Reply Postponed itself
    /// this long before T_reply_delay ends.
    pub reply_margin: u64,
    /// Extended frames on.
    pub extended: bool,
    /// The longest NPDU the station sends or takes.
    pub max_npdu: usize,
}

/// `duration` in microseconds, which it must be whole in: a timer with a
/// fraction of one would make the model inexact.
fn micros(duration: Duration) -> u64 {
    assert_eq!(
        duration.as_nanos() % 1_000,
        0,
        "the setup gave the core a timer of {duration:?}, not whole microseconds"
    );
    duration.as_micros()
}

impl Params {
    /// The parameters of `config`.
    pub fn new(config: &MstpCoreConfig) -> Self {
        let max_info_frames = u64::from(config.max_info_frames);
        let ratio = u64::from(config.multiple_frame_ratio);
        Self {
            station: config.station,
            max_master: config.max_master,
            frame_limit: (max_info_frames * ratio)
                .div_ceil(100)
                .clamp(1, max_info_frames),
            max_info_frames,
            octet_time: 10_000_000u64.div_ceil(u64::from(config.baud_rate)),
            turnaround: micros(config.t_turnaround),
            frame_abort: micros(config.t_frame_abort),
            reply_timeout: micros(config.t_reply_timeout),
            usage_timeout: micros(config.t_usage_timeout),
            reply_delay: micros(config.t_reply_delay),
            usage_delay: micros(config.t_usage_delay),
            reply_margin: micros(config.reply_transmit_margin),
            extended: config.extended_frames,
            max_npdu: if config.extended_frames {
                MAX_EXTENDED_MPDU_DATA
            } else {
                MAX_STANDARD_MPDU_DATA
            },
        }
    }

    /// The address after `address` in the ring: (address + 1) modulo
    /// (Max_Master + 1), as Clause 9.5.6 computes it.
    pub fn next(&self, address: u8) -> u8 {
        ((u16::from(address) + 1) % (u16::from(self.max_master) + 1)) as u8
    }
}

/// A frame, owned: one heard on the line or one the station sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// Its type.
    pub frame_type: FrameType,
    /// Its destination.
    pub destination: u8,
    /// Its source.
    pub source: u8,
    /// Its data, decoded.
    pub data: Vec<u8>,
}

impl Seen {
    /// An owned copy of `frame`.
    pub fn of(frame: Frame<'_>) -> Self {
        Self {
            frame_type: frame.header.frame_type,
            destination: frame.header.destination,
            source: frame.header.source,
            data: frame.data.to_vec(),
        }
    }

    /// The frame, borrowing its data.
    pub fn frame(&self) -> Frame<'_> {
        Frame::new(self.frame_type, self.destination, self.source, &self.data)
    }

    /// Decode a whole frame the station handed over, which must be exactly
    /// one valid frame.
    pub fn decode(octets: &[u8]) -> Result<Self, String> {
        let mut copy = octets.to_vec();
        match decode_frame_in_place(&mut copy) {
            Ok((frame, consumed)) if consumed == octets.len() => Ok(Self::of(frame)),
            Ok((_, consumed)) => Err(format!(
                "a transmit holds {} octets past its frame",
                octets.len() - consumed
            )),
            Err(error) => Err(format!("a transmit isn't a valid frame: {error:?}")),
        }
    }

    /// Whether this is `frame_type` addressed to `station`.
    pub fn is(&self, frame_type: FrameType, station: u8) -> bool {
        self.frame_type == frame_type && self.destination == station
    }

    /// Whether it carries an NPDU: Types 5, 6, 32 and 33.
    pub fn carries_npdu(&self) -> bool {
        is_data(self.frame_type)
    }

    /// Whether it is data that asks for a reply: Types 5 and 32.
    pub fn expects_reply(&self) -> bool {
        matches!(
            self.frame_type,
            FrameType::BACNET_DATA_EXPECTING_REPLY
                | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
        )
    }
}

/// Whether a frame of this type carries an NPDU.
pub fn is_data(frame_type: FrameType) -> bool {
    matches!(
        frame_type,
        FrameType::BACNET_DATA_EXPECTING_REPLY
            | FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    )
}

/// Builds the frames other stations send.
#[derive(Debug, Clone, Copy)]
pub struct Builder {
    /// TS.
    pub station: u8,
    /// The station this one last sent a frame to.
    pub peer: u8,
}

impl Builder {
    /// The address `address` stands for. `source` maps broadcast to 254
    /// unless `raw_source` lets 255 through.
    pub fn resolve(&self, address: Address, source: bool, raw_source: bool) -> u8 {
        match address {
            Address::This => self.station,
            Address::Broadcast if source && !raw_source => 0xFE,
            Address::Broadcast => BROADCAST_MAC,
            Address::Peer => self.peer,
            Address::Other(0xFF) if source && !raw_source => 0xFE,
            Address::Other(address) => address,
        }
    }

    /// The frame `spec` describes, with `data_wanted` asking for at least
    /// one data octet whatever the type.
    pub fn frame(&self, spec: &FrameSpec, raw_source: bool, data_wanted: bool) -> Seen {
        let long = spec.length & 0x8000 != 0;
        let length = usize::from(spec.length & 0x7FFF);
        let standard = |short: usize| {
            if long {
                length % (MAX_STANDARD_MPDU_DATA + 1)
            } else {
                length % short
            }
        };
        let cobs = if long {
            502 + length % (MAX_EXTENDED_MPDU_DATA - 501)
        } else {
            1 + length % 16
        };
        let (frame_type, length) = match spec.kind {
            Kind::Token => (FrameType::TOKEN, 0),
            Kind::PollForMaster => (FrameType::POLL_FOR_MASTER, 0),
            Kind::ReplyToPollForMaster => (FrameType::REPLY_TO_POLL_FOR_MASTER, 0),
            Kind::ReplyPostponed => (FrameType::REPLY_POSTPONED, 0),
            Kind::TestRequest => (FrameType::TEST_REQUEST, standard(8)),
            Kind::TestResponse => (FrameType::TEST_RESPONSE, standard(8)),
            Kind::DataExpectingReply => (FrameType::BACNET_DATA_EXPECTING_REPLY, standard(8)),
            Kind::DataNotExpectingReply => {
                (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, standard(8))
            }
            Kind::ExtendedExpectingReply => (FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, cobs),
            Kind::ExtendedNotExpectingReply => {
                (FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, cobs)
            }
            Kind::Reserved(raw) => (FrameType::from_raw(8 + raw % 24), standard(8)),
            Kind::ReservedCobs(raw) => (FrameType::from_raw(34 + raw % 94), cobs),
            Kind::Proprietary(raw) => (FrameType::from_raw(0x80 | raw), standard(8).max(2)),
        };
        let length = if data_wanted && frame_type.allows_data() {
            length.max(1)
        } else {
            length
        };
        Seen {
            frame_type,
            destination: self.resolve(spec.to, false, false),
            source: self.resolve(spec.from, true, raw_source),
            data: fill(length, spec.pattern),
        }
    }
}

/// The wire octets of `frame`, which must be one the line can carry: a
/// reserved COBS type goes out as Type 33 would, with its own type in the
/// header (the codec's encoder makes only Types 32 and 33).
pub fn encode(frame: &Seen) -> Vec<u8> {
    let reserved = frame.frame_type.is_cobs() && !frame.frame_type.is_extended_data();
    let as_sent = if reserved {
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    } else {
        frame.frame_type
    };
    let mut wire = vec![0; 8 + 2 * frame.data.len() + 16];
    let length = Frame::new(as_sent, frame.destination, frame.source, &frame.data)
        .encode_into(&mut wire)
        .expect("the builder makes only frames the codec encodes");
    wire.truncate(length);
    if reserved {
        wire[2] = frame.frame_type.to_raw();
        wire[7] = crc8(&wire[2..7]);
    }
    wire
}
