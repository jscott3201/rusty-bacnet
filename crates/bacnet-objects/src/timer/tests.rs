//! The Clause 12.57 state machine, write by write in each state, on a
//! hand-set monotonic clock and Device clock (#2192).

use super::*;
use crate::clock::{ClockFrame, UNSPECIFIED_DATETIME};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const MIN: u64 = 1_000;
const MAX: u64 = 100_000;
const DEFAULT: u64 = 20_000;
/// The timeout a RUNNING timer is set up with.
const SETUP: u64 = 10_000;

struct Wall(Mutex<(Date, Time)>);
impl ClockReader for Wall {
    fn read_clock(&self) -> Option<ClockFrame> {
        let (local_date, local_time) = *self.0.lock().unwrap();
        Some(ClockFrame {
            local_date,
            local_time,
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn at(minute: u8, second: u8) -> (Date, Time) {
    (
        SpecificDate::new(2026, 10, 31).unwrap().to_date(),
        Time {
            hour: 12,
            minute,
            second,
            hundredths: 0,
        },
    )
}

struct Fixture {
    timer: TimerObject,
    mono: Arc<Mutex<Duration>>,
    wall: Arc<Wall>,
    wakes: Arc<AtomicUsize>,
}

impl Fixture {
    fn new() -> Self {
        let mono = Arc::new(Mutex::new(Duration::ZERO));
        let wall = Arc::new(Wall(Mutex::new(at(0, 0))));
        let wakes = Arc::new(AtomicUsize::new(0));
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        timer.set_pres_value_range(MIN, MAX).unwrap();
        timer.set_default_timeout(DEFAULT).unwrap();
        let clock = Arc::clone(&mono);
        timer.bind_monotonic_clock_internal(Some(Arc::new(move || *clock.lock().unwrap())));
        timer.bind_clock_internal(Some(wall.clone()));
        let counter = Arc::clone(&wakes);
        timer.bind_deadline_waker_internal(Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));
        Self {
            timer,
            mono,
            wall,
            wakes,
        }
    }

    /// A timer in `state`, out of service when `oos` says so. A RUNNING one
    /// started with `SETUP` at 12:00:00 and 0 ms; an EXPIRED one ran out at
    /// 12:00:10 and 10 s.
    fn in_state(state: TimerState, oos: bool) -> Self {
        let mut fixture = Self::new();
        if state != TimerState::IDLE {
            fixture
                .write(P::PRESENT_VALUE, PropertyValue::Unsigned(SETUP))
                .unwrap();
        }
        if state == TimerState::EXPIRED {
            fixture.set_clocks(SETUP, at(0, 10));
            assert!(fixture.timer.advance_monotonic_time_internal(fixture.now()));
        }
        assert_eq!(fixture.timer.timer_state(), state);
        if oos {
            fixture
                .write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true))
                .unwrap();
        }
        fixture
    }

    fn now(&self) -> Duration {
        *self.mono.lock().unwrap()
    }

    fn set_clocks(&self, millis: u64, wall: (Date, Time)) {
        *self.mono.lock().unwrap() = Duration::from_millis(millis);
        *self.wall.0.lock().unwrap() = wall;
    }

    fn write(&mut self, property: P, value: PropertyValue) -> Result<(), Error> {
        self.timer.write_property(property, None, value, None)
    }

    fn read(&self, property: P) -> PropertyValue {
        self.timer.read_property(property, None).unwrap()
    }

    /// Every readable property, and the armed deadline.
    fn snapshot(&self) -> (Vec<(P, PropertyValue)>, Option<Duration>) {
        (
            self.timer
                .property_list()
                .iter()
                .map(|&p| (p, self.read(p)))
                .collect(),
            self.timer.next_monotonic_deadline_internal(),
        )
    }
}

fn datetime((date, time): (Date, Time)) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

fn assert_out_of_range(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY && code == ErrorCode::VALUE_OUT_OF_RANGE),
        "{result:?}"
    );
}

/// What Clause 12.57 has a write do in a state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome {
    /// Refused with PROPERTY / VALUE_OUT_OF_RANGE; nothing changes.
    Refused,
    /// Accepted; nothing changes.
    Ignored,
    /// Enter RUNNING with this timeout.
    Start(u64),
    /// Leave RUNNING for EXPIRED.
    Expire,
    /// Enter IDLE.
    Clear,
}

/// The writes under test: Present_Value in range, zero, below the range and
/// above it; Timer_Running TRUE and FALSE; Timer_State IDLE and the others.
fn writes() -> Vec<(P, PropertyValue)> {
    vec![
        (P::PRESENT_VALUE, PropertyValue::Unsigned(5_000)),
        (P::PRESENT_VALUE, PropertyValue::Unsigned(MIN)),
        (P::PRESENT_VALUE, PropertyValue::Unsigned(MAX)),
        (P::PRESENT_VALUE, PropertyValue::Unsigned(0)),
        (P::PRESENT_VALUE, PropertyValue::Unsigned(MIN - 1)),
        (P::PRESENT_VALUE, PropertyValue::Unsigned(MAX + 1)),
        (P::TIMER_RUNNING, PropertyValue::Boolean(true)),
        (P::TIMER_RUNNING, PropertyValue::Boolean(false)),
        (P::TIMER_STATE, PropertyValue::Enumerated(0)),
        (P::TIMER_STATE, PropertyValue::Enumerated(1)),
        (P::TIMER_STATE, PropertyValue::Enumerated(2)),
        (P::TIMER_STATE, PropertyValue::Enumerated(99)),
    ]
}

/// The oracle, from the state machine of Clause 12.57.
fn expected(state: TimerState, property: P, value: &PropertyValue) -> Outcome {
    let running = state == TimerState::RUNNING;
    match (property, value) {
        (P::PRESENT_VALUE, PropertyValue::Unsigned(0)) if running => Outcome::Expire,
        (P::PRESENT_VALUE, PropertyValue::Unsigned(0)) => Outcome::Ignored,
        (P::PRESENT_VALUE, PropertyValue::Unsigned(v)) if (MIN..=MAX).contains(v) => {
            Outcome::Start(*v)
        }
        (P::PRESENT_VALUE, _) => Outcome::Refused,
        (P::TIMER_RUNNING, PropertyValue::Boolean(true)) => Outcome::Start(DEFAULT),
        (P::TIMER_RUNNING, _) if running => Outcome::Expire,
        (P::TIMER_RUNNING, _) => Outcome::Ignored,
        (P::TIMER_STATE, PropertyValue::Enumerated(0)) if state == TimerState::IDLE => {
            Outcome::Ignored
        }
        (P::TIMER_STATE, PropertyValue::Enumerated(0)) => Outcome::Clear,
        _ => Outcome::Refused,
    }
}

#[test]
fn every_write_in_every_state_follows_the_state_machine() {
    let mut checked = 0;
    for state in [TimerState::IDLE, TimerState::RUNNING, TimerState::EXPIRED] {
        for oos in [false, true] {
            for (property, value) in writes() {
                let outcome = expected(state, property, &value);
                let mut fixture = Fixture::in_state(state, oos);
                // The write comes 2 s into a RUNNING timer's countdown, at
                // a wall time no setup used.
                let written_at = fixture.now().as_millis() as u64 + 2_000;
                fixture.set_clocks(written_at, at(1, 0));
                let before = fixture.snapshot();
                let wakes = fixture.wakes.load(Ordering::SeqCst);
                let result = fixture.write(property, value.clone());
                let case = format!("{state:?} oos={oos} {property:?}={value:?}");
                let transition = fixture.timer.last_state_change();
                match outcome {
                    Outcome::Refused => {
                        assert_out_of_range(result);
                        assert_eq!(fixture.snapshot(), before, "{case}");
                    }
                    Outcome::Ignored => {
                        result.unwrap();
                        assert_eq!(fixture.snapshot(), before, "{case}");
                    }
                    Outcome::Start(timeout) => {
                        result.unwrap();
                        assert_eq!(
                            transition,
                            match state {
                                TimerState::IDLE => TimerTransition::IDLE_TO_RUNNING,
                                TimerState::RUNNING => TimerTransition::RUNNING_TO_RUNNING,
                                _ => TimerTransition::EXPIRED_TO_RUNNING,
                            },
                            "{case}"
                        );
                        assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(1));
                        assert_eq!(fixture.read(P::TIMER_RUNNING), PropertyValue::Boolean(true));
                        assert_eq!(
                            fixture.read(P::PRESENT_VALUE),
                            PropertyValue::Unsigned(timeout),
                            "{case}"
                        );
                        assert_eq!(
                            fixture.read(P::INITIAL_TIMEOUT),
                            PropertyValue::Unsigned(timeout)
                        );
                        assert_eq!(fixture.read(P::UPDATE_TIME), datetime(at(1, 0)), "{case}");
                        // Every timeout here is whole seconds under an hour.
                        let seconds = 60 + timeout / 1_000;
                        let expires = at((seconds / 60) as u8, (seconds % 60) as u8);
                        assert_eq!(
                            fixture.read(P::EXPIRATION_TIME),
                            datetime(expires),
                            "{case}"
                        );
                        // In service the countdown is armed and the server's
                        // task woken; out of service it is held.
                        let deadline = Duration::from_millis(written_at + timeout);
                        assert_eq!(
                            fixture.timer.next_monotonic_deadline_internal(),
                            (!oos).then_some(deadline),
                            "{case}"
                        );
                        assert_eq!(
                            fixture.wakes.load(Ordering::SeqCst),
                            wakes + usize::from(!oos),
                            "{case}"
                        );
                    }
                    Outcome::Expire => {
                        result.unwrap();
                        assert_eq!(transition, TimerTransition::FORCED_TO_EXPIRED, "{case}");
                        assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(2));
                        assert_eq!(
                            fixture.read(P::TIMER_RUNNING),
                            PropertyValue::Boolean(false)
                        );
                        assert_eq!(fixture.read(P::PRESENT_VALUE), PropertyValue::Unsigned(0));
                        assert_eq!(fixture.read(P::UPDATE_TIME), datetime(at(1, 0)));
                        assert_eq!(fixture.read(P::EXPIRATION_TIME), datetime(at(1, 0)));
                        assert_eq!(fixture.timer.next_monotonic_deadline_internal(), None);
                    }
                    Outcome::Clear => {
                        result.unwrap();
                        assert_eq!(
                            transition,
                            if state == TimerState::RUNNING {
                                TimerTransition::RUNNING_TO_IDLE
                            } else {
                                TimerTransition::EXPIRED_TO_IDLE
                            },
                            "{case}"
                        );
                        assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(0));
                        assert_eq!(
                            fixture.read(P::TIMER_RUNNING),
                            PropertyValue::Boolean(false)
                        );
                        assert_eq!(fixture.read(P::PRESENT_VALUE), PropertyValue::Unsigned(0));
                        assert_eq!(fixture.read(P::UPDATE_TIME), datetime(at(1, 0)));
                        assert_eq!(
                            fixture.read(P::EXPIRATION_TIME),
                            datetime(UNSPECIFIED_DATETIME)
                        );
                        assert_eq!(fixture.timer.next_monotonic_deadline_internal(), None);
                    }
                }
                // Neither a write nor a refusal touches the range.
                assert_eq!(
                    fixture.read(P::MIN_PRES_VALUE),
                    PropertyValue::Unsigned(MIN)
                );
                assert_eq!(
                    fixture.read(P::MAX_PRES_VALUE),
                    PropertyValue::Unsigned(MAX)
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 3 * 2 * 12);
}

#[test]
fn a_running_timer_counts_down_and_expires_at_zero() {
    let mut fixture = Fixture::new();
    fixture
        .write(P::PRESENT_VALUE, PropertyValue::Unsigned(SETUP))
        .unwrap();
    assert_eq!(
        fixture.read(P::LAST_STATE_CHANGE),
        PropertyValue::Enumerated(1)
    );
    // Partway: the time left, rounded up to the millisecond.
    *fixture.mono.lock().unwrap() = Duration::from_micros(3_000_500);
    assert_eq!(
        fixture.read(P::PRESENT_VALUE),
        PropertyValue::Unsigned(7_000)
    );
    assert!(!fixture.timer.advance_monotonic_time_internal(fixture.now()));
    // Not due a microsecond early.
    *fixture.mono.lock().unwrap() = Duration::from_micros(9_999_999);
    assert_eq!(fixture.read(P::PRESENT_VALUE), PropertyValue::Unsigned(1));
    assert!(!fixture.timer.advance_monotonic_time_internal(fixture.now()));
    assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(1));
    fixture.set_clocks(SETUP, at(0, 10));
    assert!(fixture.timer.advance_monotonic_time_internal(fixture.now()));
    assert_eq!(
        fixture.timer.last_state_change(),
        TimerTransition::RUNNING_TO_EXPIRED
    );
    assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(2));
    assert_eq!(
        fixture.read(P::TIMER_RUNNING),
        PropertyValue::Boolean(false)
    );
    assert_eq!(fixture.read(P::PRESENT_VALUE), PropertyValue::Unsigned(0));
    assert_eq!(fixture.read(P::EXPIRATION_TIME), datetime(at(0, 10)));
    assert_eq!(fixture.read(P::UPDATE_TIME), datetime(at(0, 10)));
    assert_eq!(
        fixture.read(P::INITIAL_TIMEOUT),
        PropertyValue::Unsigned(SETUP)
    );
    assert_eq!(fixture.timer.next_monotonic_deadline_internal(), None);
    // Expiry happens once.
    *fixture.mono.lock().unwrap() = Duration::from_millis(SETUP * 2);
    assert!(!fixture.timer.advance_monotonic_time_internal(fixture.now()));
}

#[test]
fn without_a_monotonic_clock_elapsed_time_drives_the_countdown() {
    let mut timer = TimerObject::new(1, "TMR-1").unwrap();
    timer
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(5_000), None)
        .unwrap();
    assert!(!timer.advance_time_internal(Duration::from_millis(4_000)));
    assert_eq!(
        timer.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Unsigned(1_000)
    );
    assert!(timer.advance_time_internal(Duration::from_millis(1_000)));
    assert_eq!(timer.timer_state(), TimerState::EXPIRED);
    // No Device clock: the transition stamps are unspecified.
    assert_eq!(
        timer.read_property(P::UPDATE_TIME, None).unwrap(),
        datetime(UNSPECIFIED_DATETIME)
    );
}

#[test]
fn out_of_service_holds_the_countdown_and_takes_reliability() {
    let mut fixture = Fixture::new();
    fixture
        .write(P::PRESENT_VALUE, PropertyValue::Unsigned(SETUP))
        .unwrap();
    fixture.set_clocks(4_000, at(0, 4));
    let fault = PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw());
    // In service Reliability isn't the client's.
    assert!(fixture.write(P::RELIABILITY, fault.clone()).is_err());
    fixture
        .write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true))
        .unwrap();
    assert_eq!(fixture.timer.next_monotonic_deadline_internal(), None);
    fixture.set_clocks(60_000, at(1, 0));
    assert!(!fixture.timer.advance_monotonic_time_internal(fixture.now()));
    assert_eq!(
        fixture.read(P::PRESENT_VALUE),
        PropertyValue::Unsigned(6_000)
    );
    assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(1));
    fixture.write(P::RELIABILITY, fault.clone()).unwrap();
    assert_eq!(fixture.read(P::RELIABILITY), fault);
    assert_eq!(
        fixture.read(P::STATUS_FLAGS),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0101_0000],
        }
    );
    assert_out_of_range(fixture.write(
        P::RELIABILITY,
        PropertyValue::Enumerated(u32::from(u16::MAX) + 1),
    ));
    // Back in service: the countdown resumes from where it was held, and
    // Reliability returns to the object's own value.
    let wakes = fixture.wakes.load(Ordering::SeqCst);
    fixture
        .write(P::OUT_OF_SERVICE, PropertyValue::Boolean(false))
        .unwrap();
    assert_eq!(fixture.wakes.load(Ordering::SeqCst), wakes + 1);
    assert_eq!(
        fixture.timer.next_monotonic_deadline_internal(),
        Some(Duration::from_millis(66_000))
    );
    assert_eq!(
        fixture.read(P::PRESENT_VALUE),
        PropertyValue::Unsigned(6_000)
    );
    assert_eq!(
        fixture.read(P::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    fixture.set_clocks(66_000, at(1, 6));
    assert!(fixture.timer.advance_monotonic_time_internal(fixture.now()));
    assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(2));
}

#[test]
fn default_timeout_and_the_range_stay_consistent() {
    let mut fixture = Fixture::new();
    for outside in [0, MIN - 1, MAX + 1] {
        assert_out_of_range(fixture.write(P::DEFAULT_TIMEOUT, PropertyValue::Unsigned(outside)));
        assert_out_of_range(fixture.timer.set_default_timeout(outside));
    }
    assert_eq!(
        fixture.read(P::DEFAULT_TIMEOUT),
        PropertyValue::Unsigned(DEFAULT)
    );
    fixture
        .write(P::DEFAULT_TIMEOUT, PropertyValue::Unsigned(MAX))
        .unwrap();
    fixture
        .write(P::TIMER_RUNNING, PropertyValue::Boolean(true))
        .unwrap();
    assert_eq!(fixture.read(P::PRESENT_VALUE), PropertyValue::Unsigned(MAX));
    // A range must start above zero, not be inverted, and keep the default.
    for (min, max) in [(0, MAX), (MAX, MIN), (MIN, MAX - 1)] {
        assert_out_of_range(fixture.timer.set_pres_value_range(min, max));
    }
    assert_eq!(
        fixture.read(P::MIN_PRES_VALUE),
        PropertyValue::Unsigned(MIN)
    );
    assert_eq!(
        fixture.read(P::MAX_PRES_VALUE),
        PropertyValue::Unsigned(MAX)
    );
    fixture.timer.set_pres_value_range(MAX, MAX).unwrap();
    assert_out_of_range(fixture.write(P::PRESENT_VALUE, PropertyValue::Unsigned(MIN)));
}

#[test]
fn a_new_timer_is_idle_with_the_documented_range() {
    let timer = TimerObject::new(1, "TMR-1").unwrap();
    for (p, value) in [
        (P::PRESENT_VALUE, PropertyValue::Unsigned(0)),
        (P::TIMER_STATE, PropertyValue::Enumerated(0)),
        (P::TIMER_RUNNING, PropertyValue::Boolean(false)),
        (P::LAST_STATE_CHANGE, PropertyValue::Enumerated(0)),
        (P::INITIAL_TIMEOUT, PropertyValue::Unsigned(0)),
        (P::DEFAULT_TIMEOUT, PropertyValue::Unsigned(DEFAULT_TIMEOUT)),
        (P::MIN_PRES_VALUE, PropertyValue::Unsigned(1)),
        (
            P::MAX_PRES_VALUE,
            PropertyValue::Unsigned(u64::from(u32::MAX)),
        ),
        (P::UPDATE_TIME, datetime(UNSPECIFIED_DATETIME)),
        (P::EXPIRATION_TIME, datetime(UNSPECIFIED_DATETIME)),
        (
            P::OBJECT_TYPE,
            PropertyValue::Enumerated(ObjectType::TIMER.to_raw()),
        ),
    ] {
        assert_eq!(timer.read_property(p, None).unwrap(), value, "{p:?}");
    }
}

/// A RUNNING timer whose 10 s ran out at 12:00:10, with the clocks now at
/// 12:00:12 and no expiry pass run since.
fn overdue() -> Fixture {
    let mut fixture = Fixture::new();
    fixture
        .write(P::PRESENT_VALUE, PropertyValue::Unsigned(SETUP))
        .unwrap();
    fixture.set_clocks(SETUP + 2_000, at(0, 12));
    fixture
}

#[test]
fn reads_serve_a_due_expiry_before_any_pass() {
    let fixture = overdue();
    assert_eq!(fixture.read(P::TIMER_STATE), PropertyValue::Enumerated(2));
    assert_eq!(
        fixture.read(P::TIMER_RUNNING),
        PropertyValue::Boolean(false)
    );
    assert_eq!(fixture.read(P::PRESENT_VALUE), PropertyValue::Unsigned(0));
    assert_eq!(
        fixture.read(P::LAST_STATE_CHANGE),
        PropertyValue::Enumerated(4)
    );
    assert_eq!(fixture.read(P::EXPIRATION_TIME), datetime(at(0, 12)));
    assert_eq!(fixture.read(P::UPDATE_TIME), datetime(at(0, 12)));
}

#[test]
fn writes_apply_a_due_expiry_first() {
    // Zero is ignored by an expired timer: the expiry it records is the
    // countdown's, not a forced one.
    let mut fixture = overdue();
    fixture
        .write(P::PRESENT_VALUE, PropertyValue::Unsigned(0))
        .unwrap();
    assert_eq!(
        fixture.timer.last_state_change(),
        TimerTransition::RUNNING_TO_EXPIRED
    );
    assert_eq!(fixture.timer.timer_state(), TimerState::EXPIRED);
    assert_eq!(fixture.read(P::EXPIRATION_TIME), datetime(at(0, 12)));

    let mut fixture = overdue();
    fixture
        .write(P::TIMER_STATE, PropertyValue::Enumerated(0))
        .unwrap();
    assert_eq!(
        fixture.timer.last_state_change(),
        TimerTransition::EXPIRED_TO_IDLE
    );

    let mut fixture = overdue();
    fixture
        .write(P::TIMER_RUNNING, PropertyValue::Boolean(true))
        .unwrap();
    assert_eq!(
        fixture.timer.last_state_change(),
        TimerTransition::EXPIRED_TO_RUNNING
    );

    // A refused write still applies it.
    let mut fixture = overdue();
    assert_out_of_range(fixture.write(P::TIMER_STATE, PropertyValue::Enumerated(1)));
    assert_eq!(fixture.timer.timer_state(), TimerState::EXPIRED);
    assert_eq!(fixture.timer.next_monotonic_deadline_internal(), None);

    // Out of service, a countdown that already ran out isn't held at zero.
    let mut fixture = overdue();
    fixture
        .write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true))
        .unwrap();
    assert_eq!(fixture.timer.timer_state(), TimerState::EXPIRED);
    assert_eq!(fixture.timer.countdown, Countdown::Stopped);
    // The task's later pass finds nothing left to do.
    assert!(!fixture.timer.advance_monotonic_time_internal(fixture.now()));
}
