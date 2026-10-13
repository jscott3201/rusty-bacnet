//! Helpers that read token behaviour off the wire log.

use std::collections::BTreeSet;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Instant;

use crate::line::Driver;
use crate::report::WireFrame;

/// One stretch in which a station held the token, read off the wire: from a
/// clean Token addressed to it (or its own token-generation poll) to the
/// Token it sent on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    /// When the token arrived (the end of the Token frame), or when the
    /// station's generation poll started.
    pub start: Instant,
    /// Data and test frames the station sent while holding the token.
    pub info_frames: usize,
    /// Polls For Master it sent while holding it.
    pub polls: usize,
    /// Where it passed the token, once it did.
    pub passed_to: Option<u8>,
}

/// Every clean Token frame: when it started, from where, to where.
#[must_use]
pub fn token_passes(frames: &[WireFrame]) -> Vec<(Instant, u8, u8)> {
    frames
        .iter()
        .filter(|frame| frame.is_clean() && frame.frame_type() == Some(FrameType::TOKEN))
        .filter_map(|frame| Some((frame.start, frame.source()?, frame.destination()?)))
        .collect()
}

/// Stations that used a token at or after `since`: a clean Token was
/// addressed to them and the next frame on the wire was theirs. A Token sent
/// to a dead or deaf station does not count.
#[must_use]
pub fn token_users(frames: &[WireFrame], since: Instant) -> BTreeSet<u8> {
    frames
        .windows(2)
        .filter(|pair| pair[0].start >= since && pair[0].is_clean())
        .filter(|pair| pair[0].frame_type() == Some(FrameType::TOKEN))
        .filter_map(|pair| {
            let to = pair[0].destination()?;
            (pair[1].driver == Driver::Station(to)).then_some(to)
        })
        .collect()
}

/// The token holds of `station`, in order. A hold that has not ended has no
/// `passed_to`.
#[must_use]
pub fn holds(frames: &[WireFrame], station: u8) -> Vec<Hold> {
    let mut holds = Vec::new();
    let mut current: Option<Hold> = None;
    for frame in frames.iter().filter(|frame| frame.is_clean()) {
        let Some(header) = frame.header else {
            continue;
        };
        let ours = frame.driver == Driver::Station(station);
        if header.frame_type == FrameType::TOKEN && header.destination == station {
            if let Some(open) = current.take() {
                holds.push(open);
            }
            current = Some(Hold {
                start: frame.end,
                info_frames: 0,
                polls: 0,
                passed_to: None,
            });
            continue;
        }
        if ours && frame.generation {
            if let Some(open) = current.take() {
                holds.push(open);
            }
            current = Some(Hold {
                start: frame.start,
                info_frames: 0,
                polls: 1,
                passed_to: None,
            });
            continue;
        }
        let Some(hold) = current.as_mut().filter(|_| ours) else {
            continue;
        };
        match header.frame_type {
            FrameType::TOKEN => {
                hold.passed_to = Some(header.destination);
                holds.extend(current.take());
            }
            FrameType::POLL_FOR_MASTER => hold.polls += 1,
            FrameType::REPLY_TO_POLL_FOR_MASTER
            | FrameType::TEST_RESPONSE
            | FrameType::REPLY_POSTPONED => {}
            // The data types, Test_Request, and every type no constant
            // names: reserved and proprietary.
            _ => hold.info_frames += 1,
        }
    }
    holds.extend(current);
    holds
}

/// Lengths of the runs of consecutive holds without a Poll For Master that
/// lie between two holds with one: the token uses between maintenance
/// polls.
#[must_use]
pub fn holds_between_polls(holds: &[Hold]) -> Vec<usize> {
    let polled: Vec<usize> = holds
        .iter()
        .enumerate()
        .filter(|(_, hold)| hold.polls > 0)
        .map(|(index, _)| index)
        .collect();
    polled
        .windows(2)
        .map(|pair| pair[1] - pair[0] - 1)
        .filter(|&gap| gap > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use bacnet_mstp_codec::{FrameHeader, FrameType};
    use bacnet_mstp_core::Instant;

    use super::{holds, Hold};
    use crate::line::Driver;
    use crate::report::WireFrame;

    /// What a frame a station sends while it holds the token does to the
    /// hold.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Routing {
        /// It passes the token on.
        Passes,
        /// It counts as a poll.
        Poll,
        /// It counts as an information frame.
        Info,
        /// It counts as neither.
        Neither,
    }

    /// A clean frame at `at_ms` milliseconds.
    fn frame(frame_type: FrameType, source: u8, destination: u8, at_ms: u64) -> WireFrame {
        WireFrame {
            driver: Driver::Station(source),
            start: Instant::from_millis(at_ms),
            end: Instant::from_millis(at_ms + 1),
            octets: Vec::new(),
            header: Some(FrameHeader::new(frame_type, destination, source)),
            data: Vec::new(),
            collided: false,
            corrupted: false,
            truncated: false,
            generation: false,
        }
    }

    /// Every named frame type, one reserved, one reserved COBS-encoded and
    /// one proprietary, sent by station 1 between the Token it receives and
    /// the Token it passes to 2. The named list must be the codec's whole
    /// set, in order, so a type the codec names later fails here until its
    /// routing is decided, as the exhaustive match once did.
    #[test]
    fn holds_route_every_frame_type() {
        let named = [
            (FrameType::TOKEN, Routing::Passes),
            (FrameType::POLL_FOR_MASTER, Routing::Poll),
            (FrameType::REPLY_TO_POLL_FOR_MASTER, Routing::Neither),
            (FrameType::TEST_REQUEST, Routing::Info),
            (FrameType::TEST_RESPONSE, Routing::Neither),
            (FrameType::BACNET_DATA_EXPECTING_REPLY, Routing::Info),
            (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, Routing::Info),
            (FrameType::REPLY_POSTPONED, Routing::Neither),
            (
                FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
                Routing::Info,
            ),
            (
                FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
                Routing::Info,
            ),
        ];
        let all: Vec<FrameType> = FrameType::ALL_NAMED.iter().map(|&(_, t)| t).collect();
        assert_eq!(named.map(|(frame_type, _)| frame_type).to_vec(), all);
        let unnamed = [8, 34, 128].map(|raw| (FrameType::from_raw(raw), Routing::Info));

        for (frame_type, routing) in named.into_iter().chain(unnamed) {
            // A Token sent mid-hold goes to station 3, so it is told apart
            // from the closing Token to 2, which then falls outside a hold.
            let destination = if routing == Routing::Passes { 3 } else { 4 };
            let frames = [
                frame(FrameType::TOKEN, 0, 1, 0),
                frame(frame_type, 1, destination, 10),
                frame(FrameType::TOKEN, 1, 2, 20),
            ];
            let (info_frames, polls, passed_to) = match routing {
                Routing::Passes => (0, 0, 3),
                Routing::Poll => (0, 1, 2),
                Routing::Info => (1, 0, 2),
                Routing::Neither => (0, 0, 2),
            };
            let expected = Hold {
                start: Instant::from_millis(1),
                info_frames,
                polls,
                passed_to: Some(passed_to),
            };
            assert_eq!(holds(&frames, 1), [expected], "{frame_type:?}");
        }
    }
}
