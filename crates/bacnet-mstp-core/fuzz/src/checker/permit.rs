//! Whether the station may send a frame it hands over, and why.

use bacnet_mstp_codec::{BROADCAST_MAC, FrameType, MAX_STANDARD_MPDU_DATA};

use super::{After, Ask, Checker, Custody, Hold, Owed, Role};
use crate::model::{Seen, T_NO_TOKEN, T_SLOT};

impl Checker<'_> {
    /// Why the station may send `frame`, with `ps` the PS before it.
    pub(super) fn permit(&self, frame: &Seen, now: u64, ps: Option<u8>) -> Result<Role, String> {
        match self.permit_held(frame, now, ps) {
            Err(why)
                if frame.frame_type == FrameType::POLL_FOR_MASTER
                    && !matches!(self.custody, Custody::Free)
                    && self.lapsible(now) =>
            {
                self.generation(frame, now)
                    .map_err(|idle| format!("{why}; from IDLE, {idle}"))
            }
            other => other,
        }
    }

    pub(super) fn permit_held(
        &self,
        frame: &Seen,
        now: u64,
        ps: Option<u8>,
    ) -> Result<Role, String> {
        match frame.frame_type {
            FrameType::REPLY_TO_POLL_FOR_MASTER => self.answer(Ask::Poll, frame, now),
            FrameType::TEST_RESPONSE => self.answer(Ask::Test, frame, now),
            FrameType::REPLY_POSTPONED => self.answer(Ask::Reply, frame, now),
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY => {
                self.holder_data(frame).or_else(|holder| {
                    self.answer(Ask::Reply, frame, now)
                        .map_err(|answer| format!("{holder}; as an answer, {answer}"))
                })
            }
            FrameType::BACNET_DATA_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY => self.holder_data(frame),
            FrameType::TOKEN => self.token(frame, now),
            FrameType::POLL_FOR_MASTER => self.poll(frame, now, ps),
            other => Err(format!(
                "sent a frame of type {other:?}, which this station never originates"
            )),
        }
    }

    pub(super) fn answer(&self, ask: Ask, frame: &Seen, now: u64) -> Result<Role, String> {
        if ask == Ask::Reply && frame.frame_type != FrameType::REPLY_POSTPONED {
            let extended = frame.data.len() > MAX_STANDARD_MPDU_DATA;
            if frame.frame_type.is_cobs() != extended {
                return Err(format!(
                    "a reply of {} octets went as {:?}",
                    frame.data.len(),
                    frame.frame_type
                ));
            }
        }
        let answers = |owed: &&Owed| {
            owed.answerable(now)
                && owed.ask == ask
                && owed.from == frame.destination
                && match ask {
                    Ask::Poll => true,
                    Ask::Test => owed.data == frame.data,
                    Ask::Reply => {
                        frame.frame_type == FrameType::REPLY_POSTPONED
                            || owed.decided.as_deref() == Some(&frame.data[..])
                    }
                }
        };
        // The same question asked again while the station answers it (or
        // after the core gave up an answer that line activity held back) gets
        // one answer: the newest question it may answer, which gives the
        // most time.
        self.owed
            .iter()
            .rev()
            .find(answers)
            .map(|owed| Role::Answer(owed.id))
            .ok_or_else(|| {
                format!(
                    "sent {:?} to {}, which asked for no such answer",
                    frame.frame_type, frame.destination
                )
            })
    }

    pub(super) fn holder_data(&self, frame: &Seen) -> Result<Role, String> {
        let queued = self.queue.borrow().last_dequeued.clone();
        if queued.as_ref() != Some(&(frame.destination, frame.data.clone())) {
            return Err(format!(
                "a data frame that isn't the NPDU the queue handed out last ({queued:?})"
            ));
        }
        let extended = frame.data.len() > MAX_STANDARD_MPDU_DATA;
        if frame.frame_type.is_cobs() != extended {
            return Err(format!(
                "an NPDU of {} octets went as {:?}",
                frame.data.len(),
                frame.frame_type
            ));
        }
        if frame.destination != BROADCAST_MAC {
            // Clause 9.7.2: data_expecting_reply follows the NPDU's own bit.
            let expecting = frame.data.len() >= 2 && frame.data[1] & 0x04 != 0;
            if frame.expects_reply() != expecting {
                return Err("a data frame's type doesn't match its NPDU".into());
            }
        }
        self.may_send_data().map(|()| Role::Holder)
    }

    pub(super) fn may_send_data(&self) -> Result<(), String> {
        let Custody::Holding(hold) = &self.custody else {
            return Err("sent a data frame without the token".into());
        };
        match hold.after {
            After::Fresh
            | After::Reply {
                moved: true,
                data_ok: true,
            } => Ok(()),
            After::Reply { moved: true, .. } | After::Poll { moved: true, .. } if self.sole => {
                Ok(())
            }
            After::Reply { moved: true, .. } => {
                Err("sent data after a reply timeout ended its token use".into())
            }
            After::Poll { moved: true, .. } => {
                Err("sent data after an unanswered poll, though not the sole master".into())
            }
            other => Err(format!("sent data while {other:?}")),
        }
    }

    pub(super) fn token(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        let to = frame.destination;
        if to == self.params.station || to == BROADCAST_MAC {
            return Err(format!("passed the token to {to}"));
        }
        match &self.custody {
            Custody::Holding(hold) => match hold.after {
                After::Fresh
                | After::Reply { moved: true, .. }
                | After::Poll { moved: true, .. } => match self.ns {
                    // With NS unknown (NS = TS) the holder polls instead
                    // (NextStationUnknown, Clause 9.5.6.5).
                    Some(next) if next == self.params.station => {
                        Err("passed the token with no successor known".into())
                    }
                    Some(next) if next != to => Err(format!(
                        "passed the token to {to}, not to its successor {next}"
                    )),
                    _ => Ok(Role::Holder),
                },
                After::Successor(successor) if successor == to => Ok(Role::Holder),
                other => Err(format!("passed the token while {other:?}")),
            },
            Custody::Passed(pass) if pass.retried => Err("retried the token a second time".into()),
            Custody::Passed(pass) if pass.to != to => Err(format!(
                "retried the token to {to}, not to {}, its holder",
                pass.to
            )),
            Custody::Passed(_) => {
                let silence = now - self.line.last_activity;
                if silence < self.params.usage_timeout {
                    Err(format!(
                        "retried the token after {silence} us of silence, inside T_usage_timeout"
                    ))
                } else {
                    Ok(Role::Retry)
                }
            }
            Custody::Free => Err("passed a token it doesn't hold".into()),
        }
    }

    /// A poll for master, with `ps` the PS before it.
    pub(super) fn poll(&self, frame: &Seen, now: u64, ps: Option<u8>) -> Result<Role, String> {
        let station = self.params.station;
        let to = frame.destination;
        if to > self.params.max_master || to == station {
            return Err(format!(
                "polled {to} (Max_Master {})",
                self.params.max_master
            ));
        }
        let silence = now - self.line.last_activity;
        match &self.custody {
            Custody::Holding(hold) => match hold.after {
                After::Fresh
                | After::Reply { moved: true, .. }
                | After::Poll { moved: true, .. } => self.sweep(hold, to, ps).map(|_| Role::Holder),
                After::Successor(successor) if successor == station => {
                    self.sweep(hold, to, ps).map(|_| Role::Holder)
                }
                other => Err(format!("polled while {other:?}")),
            },
            Custody::Passed(pass) if !pass.retried => {
                Err("polled for a new successor before retrying the token".into())
            }
            Custody::Passed(pass) => {
                let next = self.params.next(pass.to);
                let expected = if next == station {
                    self.params.next(station)
                } else {
                    next
                };
                if silence < self.params.usage_timeout {
                    Err(format!(
                        "sought a new successor after {silence} us of silence, inside T_usage_timeout"
                    ))
                } else if to != expected {
                    Err(format!(
                        "sought a new successor for {} at {to}, not {expected}",
                        pass.to
                    ))
                } else {
                    Ok(Role::FindSuccessor)
                }
            }
            Custody::Free => self.generation(frame, now),
        }
    }

    /// GenerateToken (Clause 9.5.6.7).
    pub(super) fn generation(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        let station = self.params.station;
        let silence = now - self.line.last_activity;
        let needed = T_NO_TOKEN + T_SLOT * u64::from(station);
        if silence < needed {
            Err(format!(
                "generated a token after {silence} us of silence, short of T_no_token and {station} slots ({needed} us)"
            ))
        } else if frame.destination != self.params.next(station) {
            Err(format!(
                "generated a token with a poll to {}",
                frame.destination
            ))
        } else {
            Ok(Role::Generate)
        }
    }

    /// The sweep count after polling `to`.
    pub(super) fn sweep(&self, hold: &Hold, to: u8, ps: Option<u8>) -> Result<u64, String> {
        let station = self.params.station;
        // A poll answered from this station's own address makes it its own
        // successor, which the core takes as a reason to poll from TS + 1.
        let restart = hold.after == After::Successor(station) || hold.own_reply;
        let count = match hold.last_poll {
            // After an unanswered poll a holder with a successor passes it
            // the token (DoneWithPFM, Clause 9.5.6.8).
            Some(_) if self.ns.is_some_and(|next| next != station) => {
                return Err(format!(
                    "polled {to} after an unanswered poll, with {:?} to pass the token to",
                    self.ns
                ));
            }
            Some(previous) => {
                let next = self.params.next(previous);
                if next != station && to == next {
                    hold.sweep + 1
                } else if (next == station || restart) && to == self.params.next(station) {
                    1
                } else {
                    return Err(format!(
                        "polled {to} right after {previous}: a sweep goes up one address at a time"
                    ));
                }
            }
            // A token use's first poll goes to PS + 1 (SendMaintenancePFM),
            // or to TS + 1 once PS was reset or NS is unknown, and never to
            // NS, at which the maintenance sweep ends (Clause 9.5.6.5).
            None => {
                let next = |address: Option<u8>| address.map(|address| self.params.next(address));
                if ps.is_some_and(|_| Some(to) != next(ps)) && to != self.params.next(station) {
                    return Err(format!(
                        "polled {to} first in a token use, not PS + 1 ({:?}) or TS + 1",
                        next(ps)
                    ));
                }
                if self.ns.is_some_and(|ns| ns != station && ns == to) {
                    return Err(format!("polled {to}, its own successor"));
                }
                1
            }
        };
        if count > u64::from(self.params.max_master) {
            return Err(format!(
                "polled {count} addresses in one sweep, more than Max_Master ({})",
                self.params.max_master
            ));
        }
        Ok(count)
    }
}
