//! The shed state machine of Clause 12.28 (Figure 12-5), and the route by
//! which the application that sheds the loads answers it.
//!
//! Writes and time drive the machine. A write of Start_Time is the request
//! that leaves SHED_INACTIVE; a write of Requested_Shed_Level, Start_Time,
//! Shed_Duration or Duty_Window sends a request in either shedding state back
//! to SHED_REQUEST_PENDING, which decides at once where it goes next, and so
//! does a Device clock set back before Start_Time. Time is the Device clock,
//! which the server reads on each write and on its once-a-second operation
//! pass. Whether the loads can meet the request, and
//! the levels they reach, come from the application's [`LoadShedder`].

use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::ShedState;
use bacnet_types::primitives::{Date, Time};

use super::{LoadControlObject, check_requested_level, choice_default};
use crate::clock::{self, UNSPECIFIED_DATETIME};

/// Hundredths of a second in a minute, the unit of Shed_Duration and
/// Duty_Window.
const HUNDREDTHS_PER_MINUTE: i64 = 60 * 100;

/// The application side of a Load Control object: the loads it sheds, and
/// what it reports about them (Clause 12.28 leaves both to the device).
///
/// Install one with [`LoadControlObject::set_shedder`]. The object asks it
/// through [`shed`](Self::shed) each time it looks at a request: on every
/// write or setter call that changes the request, and on each pass of the
/// server's operation task (once a second) while a request is pending or
/// being shed. It tells it through [`stop`](Self::stop) when a request ends.
///
/// Both run while the host holds the object exclusively (the server holds
/// the database's write guard): return quickly, never block, and never reach
/// for the object database. A panic is caught only with `std` and panics
/// that unwind: then a `shed` that panics counts as nothing shed, and a
/// `stop` that panics is logged and the request ends anyway. Without `std`
/// the panic goes on up to the caller, and with `panic = "abort"` it ends
/// the process.
pub trait LoadShedder: Send + Sync {
    /// Look at `request` and say how the loads stand against it.
    ///
    /// Before Start_Time ([`ShedRequest::started`] false) the application may
    /// begin shedding early so that it complies by Start_Time; from Start_Time
    /// on, it sheds the requested level for the rest of the request.
    fn shed(&self, request: &ShedRequest) -> ShedStatus;

    /// The request ended: it ran its Shed_Duration, a client cancelled it,
    /// Enable went FALSE, or the object left its database, removed from it
    /// or replaced by another object (#2479). Stop shedding. The default
    /// does nothing.
    ///
    /// Only a request the object has offered to [`shed`](Self::shed) is
    /// stopped: one cancelled by the write that started it never reaches
    /// the application. Dropping the object, or a copy of it, outside its
    /// database calls nothing.
    fn stop(&self) {}
}

/// A shed request as a Load Control object holds it, for its
/// [`LoadShedder`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ShedRequest {
    /// Requested_Shed_Level: the shed asked for.
    pub requested: BACnetShedLevel,
    /// For a LEVEL request, the position in Shed_Levels (the BACnet array
    /// index minus one) of the element whose action it selects: the element
    /// with the requested value, or else the one with the nearest lower value
    /// (Clause 12.28.18), the first such when values repeat. `None` for
    /// another choice, or when no element is at or below the level.
    pub shed_level: Option<usize>,
    /// Start_Time: when the loads must comply, and the first duty window
    /// begins.
    pub start_time: (Date, Time),
    /// Shed_Duration: how long the shed lasts from Start_Time, in minutes.
    pub shed_duration: u64,
    /// Duty_Window: the window over which compliance is judged, in minutes.
    pub duty_window: u64,
    /// Whether Start_Time has come.
    pub started: bool,
}

/// How the loads stand against a [`ShedRequest`], as the application reports
/// it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ShedStatus {
    /// Where the loads stand.
    pub progress: ShedProgress,
    /// Expected_Shed_Level: the shed the application expects to reach by
    /// Start_Time.
    pub expected: BACnetShedLevel,
    /// Actual_Shed_Level: the shed reached, averaged over the last duty
    /// window.
    pub actual: BACnetShedLevel,
}

impl ShedStatus {
    /// A status with these values.
    ///
    /// Both levels are in the request's units, the same choice as
    /// Requested_Shed_Level (Clauses 12.28.16 and 12.28.17), and in its
    /// range: a percent of at most 100, an amount that is finite and not
    /// negative. The object serves its choice's no-shed default in place of a
    /// level that isn't.
    pub fn new(progress: ShedProgress, expected: BACnetShedLevel, actual: BACnetShedLevel) -> Self {
        Self {
            progress,
            expected,
            actual,
        }
    }

    /// Nothing shed: [`ShedProgress::Waiting`], with both levels at the
    /// no-shed default of `requested`'s choice (Table 12-33). The object
    /// takes this from a request when no [`LoadShedder`] is installed.
    pub fn idle(requested: &BACnetShedLevel) -> Self {
        let none = choice_default(requested);
        Self::new(ShedProgress::Waiting, none.clone(), none)
    }
}

/// Where the loads stand against a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShedProgress {
    /// Not shedding: waiting for Start_Time.
    Waiting,
    /// Shedding, without meeting the requested level.
    Shedding,
    /// Meeting the requested level.
    Compliant,
}

/// What prompted a look at the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Prompt {
    /// A write of Start_Time.
    StartTime,
    /// A write of Requested_Shed_Level, Shed_Duration or Duty_Window.
    Parameters,
    /// Anything else: time passing, Enable, or a change the application
    /// may answer differently, such as a write of Shed_Levels.
    Look,
    /// The object is leaving its database (#2479).
    Retire,
}

/// The readable state a look may change, to tell whether it did.
type Observed = (
    ShedState,
    BACnetShedLevel,
    BACnetShedLevel,
    BACnetShedLevel,
    (Date, Time),
    u64,
    u64,
);

impl LoadControlObject {
    /// Run the state machine after `prompt`, and return whether any property
    /// a client reads changed.
    pub(super) fn settle(&mut self, prompt: Prompt) -> bool {
        let before = self.observed();
        self.step(prompt);
        // Clauses 12.28.16 and 12.28.17: with no shed, both levels read the
        // requested choice's default.
        if self.state == ShedState::SHED_INACTIVE {
            let none = choice_default(&self.requested_shed_level);
            self.expected_shed_level = none.clone();
            self.actual_shed_level = none;
        }
        before != self.observed()
    }

    fn observed(&self) -> Observed {
        (
            self.state,
            self.requested_shed_level.clone(),
            self.expected_shed_level.clone(),
            self.actual_shed_level.clone(),
            self.start_time,
            self.shed_duration,
            self.duty_window,
        )
    }

    fn step(&mut self, prompt: Prompt) {
        // An object removed or replaced takes its request with it, and the
        // application stops the loads it shed for it (#2479).
        if prompt == Prompt::Retire {
            if self.state != ShedState::SHED_INACTIVE {
                self.end_request();
            }
            return;
        }
        // Clause 12.28.14: a disabled object ends any request and takes no
        // new one. Start_Time and Shed_Duration hold their idle values
        // while it is disabled (their setters keep nothing then), so a
        // Shed_Duration staged before Enable went FALSE goes too; the
        // level and Duty_Window stay as configuration.
        if !self.enable {
            if self.state == ShedState::SHED_INACTIVE {
                self.shed_duration = 0;
            } else {
                self.end_request();
            }
            return;
        }
        match (self.state, prompt) {
            // ReceiveShedRequest: only a Start_Time write starts a request.
            // The unspecified Start_Time, which would cancel one at once,
            // starts none: while idle it changes nothing.
            (ShedState::SHED_INACTIVE, Prompt::StartTime)
                if self.start_time != UNSPECIFIED_DATETIME =>
            {
                self.state = ShedState::SHED_REQUEST_PENDING;
            }
            (ShedState::SHED_INACTIVE, _) => return,
            // SuccessfulShedReconfigured and UnsuccessfulShedReconfigured.
            (
                ShedState::SHED_COMPLIANT | ShedState::SHED_NON_COMPLIANT,
                Prompt::StartTime | Prompt::Parameters,
            ) => self.state = ShedState::SHED_REQUEST_PENDING,
            _ => {}
        }
        // One reading of the clock serves the whole look.
        let now = self.now();
        if self.state == ShedState::SHED_REQUEST_PENDING {
            self.decide_pending(now);
        } else {
            self.follow_shed(now);
        }
    }

    /// SHED_REQUEST_PENDING at the moment `now`: cancel the request, keep
    /// waiting, or start shedding.
    fn decide_pending(&mut self, now: Option<i64>) {
        // CancelShed: a default level or an unspecified Start_Time cancels.
        let start = clock::local_moment(self.start_time);
        let cancelled = self.requested_shed_level == choice_default(&self.requested_shed_level);
        let Some(start) = start.filter(|_| !cancelled) else {
            self.end_request();
            return;
        };
        // Without a Device clock the object can't tell when Start_Time
        // comes, so the request waits. The levels last reported hold, unless
        // a new requested choice has made them the wrong units.
        let Some(now) = now else {
            self.expected_shed_level = self.in_requested_units(self.expected_shed_level.clone());
            self.actual_shed_level = self.in_requested_units(self.actual_shed_level.clone());
            return;
        };
        // CancelShed: a request whose end has passed is ignored. The
        // instant of Start_Time plus Shed_Duration counts as passed, by
        // choice: the shed runs up to it, not through it.
        if now >= self.end_of(start) {
            self.end_request();
            return;
        }
        let started = now >= start;
        let progress = self.ask(started);
        self.state = match (progress, started) {
            // AbleToMeetShed, at Start_Time or early.
            (ShedProgress::Compliant, _) => ShedState::SHED_COMPLIANT,
            // CannotMeetShed.
            (_, true) => ShedState::SHED_NON_COMPLIANT,
            // PrepareToShed.
            (ShedProgress::Shedding, false) => ShedState::SHED_NON_COMPLIANT,
            // ReconfigurePending: wait for Start_Time.
            (_, false) => ShedState::SHED_REQUEST_PENDING,
        };
    }

    /// SHED_COMPLIANT or SHED_NON_COMPLIANT at the moment `now`: finish at
    /// the request's end, or follow the application between the two. While
    /// the clock can't be read the state and levels hold.
    fn follow_shed(&mut self, now: Option<i64>) {
        let (Some(start), Some(now)) = (clock::local_moment(self.start_time), now) else {
            return;
        };
        // The Device clock moved back before Start_Time (it was set, or
        // daylight saving ended): look at the request again as if Start_Time
        // had just been written, through SHED_REQUEST_PENDING.
        if now < start {
            self.state = ShedState::SHED_REQUEST_PENDING;
            self.decide_pending(Some(now));
            return;
        }
        // FinishedSuccessfulShed and FinishedUnsuccessfulShed, at the same
        // instant CancelShed takes as passed.
        if now >= self.end_of(start) {
            self.end_request();
            return;
        }
        // CanNowComplyWithShed and CanNoLongerComplyWithShed.
        self.state = match self.ask(now >= start) {
            ShedProgress::Compliant => ShedState::SHED_COMPLIANT,
            _ => ShedState::SHED_NON_COMPLIANT,
        };
    }

    /// Stop shedding and return to SHED_INACTIVE, with the request's
    /// properties back at their idle values: Requested_Shed_Level at its
    /// choice's default, Start_Time unspecified, Shed_Duration zero and
    /// Duty_Window the agreed value (Clauses 12.28.10 to 12.28.13). The
    /// clause names these resets for a request that ran its course; the
    /// object makes them however the request ends, so a request is never
    /// left half there. The application is told to stop only when it was
    /// asked about the request.
    fn end_request(&mut self) {
        if core::mem::take(&mut self.offered)
            && let Some(shedder) = &self.shedder
            && crate::catch_panic(|| shedder.stop()).is_none()
        {
            let oid = self.oid;
            log_warn!(object = %oid, "A Load Control's shedder panicked as its request ended");
        }
        self.state = ShedState::SHED_INACTIVE;
        self.requested_shed_level = choice_default(&self.requested_shed_level);
        self.start_time = UNSPECIFIED_DATETIME;
        self.shed_duration = 0;
        self.duty_window = self.agreed_duty_window;
    }

    /// The Device clock's local moment now, if it has one.
    fn now(&self) -> Option<i64> {
        clock::current_datetime(self.clock.as_deref()).and_then(clock::local_moment)
    }

    /// The moment a request starting at `start` ends.
    fn end_of(&self, start: i64) -> i64 {
        let minutes = i64::try_from(self.shed_duration).unwrap_or(i64::MAX);
        start.saturating_add(minutes.saturating_mul(HUNDREDTHS_PER_MINUTE))
    }

    /// Ask the application how the loads stand, and serve the levels it
    /// reports. From here on the request counts as offered to it.
    fn ask(&mut self, started: bool) -> ShedProgress {
        self.offered = true;
        let request = ShedRequest {
            requested: self.requested_shed_level.clone(),
            shed_level: match self.requested_shed_level {
                BACnetShedLevel::Level(level) => selected_shed_level(&self.shed_levels, level),
                _ => None,
            },
            start_time: self.start_time,
            shed_duration: self.shed_duration,
            duty_window: self.duty_window,
            started,
        };
        let status = match &self.shedder {
            Some(shedder) => crate::catch_panic(|| shedder.shed(&request)).unwrap_or_else(|| {
                let oid = self.oid;
                log_warn!(object = %oid, "A Load Control's shedder panicked; counting nothing shed");
                ShedStatus::idle(&request.requested)
            }),
            None => ShedStatus::idle(&request.requested),
        };
        self.expected_shed_level = self.in_requested_units(status.expected);
        self.actual_shed_level = self.in_requested_units(status.actual);
        status.progress
    }

    /// `level` when it is in Requested_Shed_Level's choice and range,
    /// otherwise that choice's default.
    fn in_requested_units(&self, level: BACnetShedLevel) -> BACnetShedLevel {
        let same_choice =
            core::mem::discriminant(&level) == core::mem::discriminant(&self.requested_shed_level);
        if same_choice && check_requested_level(&level).is_ok() {
            level
        } else {
            choice_default(&self.requested_shed_level)
        }
    }
}

/// The position in `levels` of the element a LEVEL request for `requested`
/// selects (Clause 12.28.18): the greatest value at or below it, the first
/// such when values repeat.
fn selected_shed_level(levels: &[u64], requested: u64) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for (position, &value) in levels.iter().enumerate() {
        if value <= requested && best.is_none_or(|(_, held)| value > held) {
            best = Some((position, value));
        }
    }
    best.map(|(position, _)| position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_selects_its_element_or_the_nearest_lower_one() {
        let levels = [2, 5, 5, 9];
        assert_eq!(selected_shed_level(&levels, 5), Some(1));
        assert_eq!(selected_shed_level(&levels, 7), Some(1));
        assert_eq!(selected_shed_level(&levels, 9), Some(3));
        assert_eq!(selected_shed_level(&levels, u64::MAX), Some(3));
        assert_eq!(selected_shed_level(&levels, 2), Some(0));
        assert_eq!(selected_shed_level(&levels, 1), None);
        assert_eq!(selected_shed_level(&[], 3), None);
        // The order of the array doesn't change the answer.
        assert_eq!(selected_shed_level(&[9, 2, 5], 6), Some(2));
    }
}
