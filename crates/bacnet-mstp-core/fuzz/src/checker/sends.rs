//! The station's own frames from hand-over to completion, and what it hands
//! up.

use bacnet_mstp_codec::{BROADCAST_MAC, FrameType};
use bacnet_mstp_core::RequestId;

use super::{After, Ask, Checker, Custody, Handover, Hold, Owed, Pass, Role};
use crate::model::Seen;
use crate::model::{MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT, N_MIN_OCTETS, N_POLL};

/// How the core handed up received data.
#[derive(Debug, Clone, Copy)]
pub enum Delivered {
    /// As an NPDU.
    Npdu,
    /// Whole, as a frame of this type.
    Frame(FrameType),
}

impl Checker<'_> {
    /// The adapter confirmed the start of the frame it holds.
    pub fn started(&mut self, now: u64) {
        self.now = now;
        let before = self.custody;
        let Some(handover) = self.handover.take() else {
            self.fail("a start confirmed for no frame handed over");
        };
        let silence = now - self.line.last_activity;
        if silence < self.params.turnaround {
            self.fail(&format!(
                "started {:?} {silence} us after line activity, inside T_turnaround",
                handover.frame.frame_type
            ));
        }
        let frame = &handover.frame;
        match handover.role {
            Role::Answer(id) => {
                let Some(index) = self.owed.iter().position(|owed| owed.id == id) else {
                    self.fail(&format!("answered with {frame:?} twice"));
                };
                let owed = self.owed.remove(index);
                // It answers every handed-over question of its kind.
                self.owed.retain(|other| {
                    !(other.handed && other.ask == owed.ask && other.from == owed.from)
                });
                let window = match owed.ask {
                    Ask::Poll => MIN_USAGE_TIMEOUT,
                    Ask::Test | Ask::Reply => MIN_REPLY_TIMEOUT,
                };
                if now + self.params.octet_time >= owed.at + window {
                    self.fail(&format!(
                        "an answer to {} starts {} us after the question, too late for its first octet to beat the asker's shortest timeout",
                        owed.from,
                        now - owed.at
                    ));
                }
            }
            Role::Holder => self.start_holder(&handover),
            Role::Retry => match self.custody {
                Custody::Passed(pass) if !pass.retried && pass.to == frame.destination => {}
                _ => self.fail("started a token retry after its successor used the token"),
            },
            Role::FindSuccessor | Role::Generate => {
                let idle = matches!(self.custody, Custody::Free) || self.lapsible(now);
                let still = match (handover.role, &self.custody) {
                    (Role::FindSuccessor, Custody::Passed(pass)) => pass.retried,
                    (Role::Generate, _) if idle => {
                        self.line.frames == handover.frames
                            && self.line.events - handover.events <= N_MIN_OCTETS
                    }
                    _ => false,
                };
                if !still {
                    self.fail(&format!(
                        "started a {:?} poll after the line showed another station active",
                        handover.role
                    ));
                }
                self.custody = Custody::Holding(Hold {
                    frames: 0,
                    after: After::Sending,
                    last_poll: Some(frame.destination),
                    sweep: 1,
                    own_reply: false,
                });
            }
        }
        self.line.started();
        self.in_flight = Some(handover);
        self.track(before, now);
    }

    fn start_holder(&mut self, handover: &super::Handover) {
        let frame = &handover.frame;
        let checked = match frame.frame_type {
            FrameType::TOKEN => self.token(frame, self.now).map(|_| 0),
            FrameType::POLL_FOR_MASTER => self.poll(frame, self.now, handover.ps).and_then(|_| {
                let Custody::Holding(hold) = &self.custody else {
                    unreachable!("a holder's poll was permitted");
                };
                self.sweep(hold, frame.destination, handover.ps)
            }),
            _ => self.may_send_data().map(|()| 0),
        };
        let sweep = checked.unwrap_or_else(|why| self.fail(&format!("at its start, {why}")));
        let (sole, limit) = (self.sole, self.params.frame_limit);
        let max_info_frames = self.params.max_info_frames;
        let Custody::Holding(hold) = &mut self.custody else {
            unreachable!("a holder's frame was permitted");
        };
        match frame.frame_type {
            FrameType::TOKEN => {
                hold.last_poll = None;
                self.sole = false;
                self.ns = Some(frame.destination);
            }
            FrameType::POLL_FOR_MASTER => {
                hold.last_poll = Some(frame.destination);
                hold.sweep = sweep;
            }
            _ => {
                if matches!(hold.after, After::Poll { .. }) {
                    // A sole master's poll went unanswered: a new token use.
                    hold.frames = 0;
                }
                hold.frames += 1;
                hold.last_poll = None;
                let bound = if sole { limit * (N_POLL + 1) } else { limit };
                if hold.frames > bound {
                    let frames = hold.frames;
                    self.fail(&format!(
                        "sent {frames} data frames in one token use: Max_Info_Frames is {max_info_frames}, its local share {limit}"
                    ));
                }
                self.queue.borrow_mut().last_dequeued = None;
            }
        }
        if let Custody::Holding(hold) = &mut self.custody {
            hold.after = After::Sending;
            hold.own_reply = false;
        }
    }

    /// The adapter reported the frame's last stop bit.
    pub fn completed(&mut self, now: u64) {
        self.now = now;
        let Some(sent) = self.in_flight.take() else {
            self.fail("a completion for no frame started");
        };
        self.line.driven_until(now);
        let before = self.custody;
        self.complete_role(sent);
        self.track(before, now);
    }

    fn complete_role(&mut self, sent: super::Handover) {
        let frame = sent.frame;
        match sent.role {
            Role::Answer(_) => {}
            Role::Retry => {
                self.custody = Custody::Passed(Pass {
                    to: frame.destination,
                    retried: true,
                    base: self.line.events,
                });
            }
            Role::Holder | Role::FindSuccessor | Role::Generate => {
                if frame.frame_type == FrameType::TOKEN {
                    self.custody = Custody::Passed(Pass {
                        to: frame.destination,
                        retried: false,
                        base: self.line.events,
                    });
                    return;
                }
                let Custody::Holding(hold) = &mut self.custody else {
                    self.fail("a holder's frame completed after it lost the token");
                };
                hold.after = if frame.frame_type == FrameType::POLL_FOR_MASTER {
                    After::Poll {
                        to: frame.destination,
                        moved: false,
                    }
                } else if frame.expects_reply() && frame.destination != BROADCAST_MAC {
                    After::Reply {
                        moved: false,
                        data_ok: false,
                    }
                } else {
                    After::Fresh
                };
            }
        }
    }

    /// A frame cut short after its start, by the adapter or the core's
    /// watchdog. Each octet sent restarts the silence (Clause 9.5.5.1), and a
    /// holder goes back to IDLE without the token.
    pub fn aborted(&mut self, now: u64) {
        self.now = now;
        let Some(sent) = self.in_flight.take() else {
            self.fail("an abort for no frame started");
        };
        self.line.driven_until(now);
        let before = self.custody;
        if !matches!(sent.role, Role::Answer(_)) {
            self.custody = Custody::Free;
        }
        self.track(before, now);
    }

    /// The core took back the frame it handed over before its start.
    pub fn withdrawn(&mut self) {
        match self.handover.take().map(|handover| handover.role) {
            // Taken back at its limit, or by the watchdog.
            Some(Role::Answer(id)) => self.answer_released(id),
            Some(_) => self.lapsed = true,
            None => {}
        }
        self.deferred = None;
    }

    /// The core refused a start as too soon after line activity; the frame
    /// waits again.
    pub fn deferred(&mut self, now: u64) {
        self.now = now;
        let silence = now - self.line.last_activity;
        if silence >= self.params.turnaround {
            self.fail(&format!(
                "a start refused as deferred after {silence} us of silence, T_turnaround or more"
            ));
        }
        self.deferred = self.handover.take();
        if let Some(Handover {
            role: Role::Answer(id),
            ..
        }) = self.deferred
        {
            // Back to waiting: the core may now drop it at its limit
            // without a word on the line.
            self.answer_unhanded(id);
        }
    }

    /// The core refused a start as too late for the asker.
    pub fn expired(&mut self, now: u64) {
        self.now = now;
        match self.handover.take() {
            Some(Handover {
                role: Role::Answer(id),
                ..
            }) => self.answer_released(id),
            other => self.fail(&format!("a start refused as expired for {other:?}")),
        }
    }

    /// Whether a frame is on the wire.
    pub fn sending(&self) -> bool {
        self.in_flight.is_some()
    }

    /// The core handed up received data.
    pub fn delivered(
        &mut self,
        how: Delivered,
        source: u8,
        destination: u8,
        data: &[u8],
        request: Option<RequestId>,
    ) {
        let station = self.params.station;
        let asks = |seen: &Seen| seen.expects_reply() && seen.destination == station;
        let carried = |seen: &Seen| {
            seen.source == source
                && seen.destination == destination
                && seen.data == data
                && match how {
                    Delivered::Npdu => seen.carries_npdu(),
                    Delivered::Frame(frame_type) => {
                        !seen.carries_npdu() && seen.frame_type == frame_type
                    }
                }
        };
        // An NPDU doesn't say which data type brought it: the earliest frame
        // that carried it whose type matches the request id, if any does.
        let found = self
            .deliverable
            .iter()
            .position(|seen| carried(seen) && asks(seen) == request.is_some());
        let Some(index) = found else {
            let other = self.deliverable.iter().find(|seen| carried(seen));
            self.fail(&format!(
                "delivered {how:?} from {source} to {destination} ({} octets) with request id {request:?}, which no frame heard since the last delivery matches (nearest {:?})",
                data.len(),
                other.map(|seen| seen.frame_type)
            ));
        };
        self.deliverable.drain(..=index);
        if let Some(id) = request {
            // The request the core began: the oldest one it was free to
            // take, since it discards repeats while busy with it; failing
            // that, the newest it may still answer.
            let now = self.now;
            let candidate = |owed: &Owed| {
                owed.ask == Ask::Reply
                    && owed.from == source
                    && owed.request.is_none()
                    && owed.data == data
                    && owed.answerable(now)
            };
            let index = self
                .owed
                .iter()
                .position(|owed| candidate(owed) && owed.required)
                .or_else(|| self.owed.iter().rposition(candidate));
            if let Some(index) = index {
                self.owed[index].request = Some(id);
            }
        }
    }
}
