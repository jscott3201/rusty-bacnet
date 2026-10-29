//! What the station was asked, and the answers it owes.
//!
//! A poll, a Test_Request or a data request for this station is owed an
//! answer (Clauses 9.5.6.2, 9.5.6.9) when the station was free to give it:
//! not holding the token, in whose states such a frame is dropped or
//! discarded, and not still answering an earlier question, which it finishes
//! first. An owed answer has to go out once it exists (at once for a poll or
//! a Test_Request; for a data request once the application has decided, or
//! at T_reply_delay less the core's margin, when Reply Postponed goes in its
//! place) and the line has been quiet for T_turnaround, until the asker may
//! stop waiting. The checker looks each time the adapter services a
//! deadline or the application decides: an owed answer the core could have
//! sent then and never handed over is a fault. One the application
//! abandoned, or the core gave up on at its limit, is not.
//!
//! A question keeps the station busy until its answer starts or the core
//! lets it go: the application abandons it, the core refuses a late start
//! or takes the frame back, or a deadline is serviced past the limit. Time
//! alone ends nothing, since the core acts only when called.

use bacnet_mstp_core::RequestId;

use super::{Ask, Checker, Custody, Owed};
use crate::model::{MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT, Seen};

/// A decision the core took for a request.
#[derive(Debug, Clone, Copy)]
pub enum Decided<'a> {
    /// Answer with this data.
    Data(&'a [u8]),
    /// Reply Postponed, or a cancellation, which sends it too.
    Postponed,
    /// No answer at all.
    Abandoned,
}

impl Owed {
    /// The asker's shortest wait, and the latest an answer should start.
    fn window(&self) -> u64 {
        match self.ask {
            Ask::Poll => MIN_USAGE_TIMEOUT,
            Ask::Test | Ask::Reply => MIN_REPLY_TIMEOUT,
        }
    }

    /// Whether an answer may still go to it: the asker may still wait.
    pub(super) fn answerable(&self, now: u64) -> bool {
        !self.released && self.at + self.window() > now
    }
}

impl Checker<'_> {
    /// `frame` asks this station for an answer.
    pub(super) fn asked(&mut self, ask: Ask, frame: &Seen, now: u64) {
        let busy = matches!(self.custody, Custody::Holding(_)) || self.answering();
        self.owed.retain(|owed| !owed.released);
        self.owed.push(Owed {
            id: self.next_owed,
            ask,
            from: frame.source,
            at: now,
            data: frame.data.clone(),
            request: None,
            decided: None,
            required: !busy,
            made: ask != Ask::Reply,
            handed: false,
            released: false,
        });
        self.next_owed += 1;
    }

    /// Whether the station may still be answering a question.
    pub(super) fn answering(&self) -> bool {
        self.owed.iter().any(|owed| !owed.released)
    }

    /// The latest start the core allows an answer: the asker's shortest
    /// wait less one octet and a 1 ms guard, but never before the end of
    /// T_reply_delay (T_usage_delay for a poll).
    fn give_up(&self, owed: &Owed) -> u64 {
        let send_by = match owed.ask {
            Ask::Poll => self.params.usage_delay,
            Ask::Test | Ask::Reply => self.params.reply_delay,
        };
        (owed.at + owed.window() - self.params.octet_time - 1_000).max(owed.at + send_by)
    }

    /// Check that every answer the core could send by `now` was handed over:
    /// after a deadline service, any it owes; after the application's
    /// decision on request `decided`, the answer that decision made.
    pub fn answers_due(&mut self, now: u64, decided: Option<RequestId>) {
        if decided.is_none() {
            // A deadline serviced past an answer's limit lets it go, unless
            // the adapter holds it.
            let limits: Vec<u64> = self.owed.iter().map(|owed| self.give_up(owed)).collect();
            for (owed, limit) in self.owed.iter_mut().zip(limits) {
                if !owed.handed && now >= limit {
                    owed.released = true;
                }
            }
        }
        if now - self.line.last_activity < self.params.turnaround {
            return;
        }
        let decide_by = self.params.reply_delay - self.params.reply_margin;
        for owed in &self.owed {
            if !owed.required || owed.handed || owed.released || now >= self.give_up(owed) {
                continue;
            }
            let due = match decided {
                Some(id) => owed.made && owed.request == Some(id),
                None => owed.made || now >= owed.at + decide_by,
            };
            if due {
                self.fail(&format!(
                    "owes {} an answer to its {:?} of {} us ago, and the line has been quiet for T_turnaround, but nothing went out",
                    owed.from,
                    owed.ask,
                    now - owed.at
                ));
            }
        }
    }

    /// The core handed over the answer to `id`, which answers every live
    /// question of its kind from that station.
    pub(super) fn answer_handed(&mut self, id: u64, now: u64) {
        let Some(answered) = self.owed.iter().find(|owed| owed.id == id).cloned() else {
            return;
        };
        for owed in &mut self.owed {
            if owed.ask == answered.ask && owed.from == answered.from && owed.answerable(now) {
                owed.handed = true;
            }
        }
    }

    /// The core took back the answer to `id` to wait again.
    pub(super) fn answer_unhanded(&mut self, id: u64) {
        let Some(answered) = self.owed.iter().find(|owed| owed.id == id).cloned() else {
            return;
        };
        for owed in &mut self.owed {
            if owed.ask == answered.ask && owed.from == answered.from {
                owed.handed = false;
            }
        }
    }

    /// The core gave up the answer to `id` unsent.
    pub(super) fn answer_released(&mut self, id: u64) {
        if let Some(owed) = self.owed.iter_mut().find(|owed| owed.id == id) {
            owed.released = true;
        }
    }

    /// The application decided request `id`, and the core took it.
    pub fn decided(&mut self, id: RequestId, decided: Decided<'_>) {
        let Some(owed) = self.owed.iter_mut().find(|owed| owed.request == Some(id)) else {
            return;
        };
        match decided {
            Decided::Data(data) => {
                owed.decided = Some(data.to_vec());
                owed.made = true;
            }
            Decided::Postponed => owed.made = true,
            Decided::Abandoned => {
                // The repeats the core discarded while busy go with it.
                let (from, data) = (owed.from, owed.data.clone());
                for owed in &mut self.owed {
                    if owed.ask == Ask::Reply && owed.from == from && owed.data == data {
                        owed.released = true;
                    }
                }
            }
        }
    }
}
