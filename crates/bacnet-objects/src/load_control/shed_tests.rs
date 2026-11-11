//! The shed state machine (Clause 12.28, Figure 12-5), driven by writes and
//! by a hand-set Device clock.

use super::*;
use crate::clock::ClockFrame;
use crate::traits::{Clocks, Timed};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use std::sync::Mutex;
use std::time::Duration;

use PropertyIdentifier as P;

const INACTIVE: ShedState = ShedState::SHED_INACTIVE;
const PENDING: ShedState = ShedState::SHED_REQUEST_PENDING;
const COMPLIANT: ShedState = ShedState::SHED_COMPLIANT;
const NON_COMPLIANT: ShedState = ShedState::SHED_NON_COMPLIANT;

/// 2026-11-11, a Wednesday.
const DAY: Date = Date {
    year: 126,
    month: 11,
    day: 11,
    day_of_week: 3,
};

/// `minutes` past 10:00 on [`DAY`].
fn at(minutes: u8) -> (Date, Time) {
    (
        DAY,
        Time {
            hour: 10 + minutes / 60,
            minute: minutes % 60,
            second: 0,
            hundredths: 0,
        },
    )
}

/// A Device clock the test sets by hand.
struct TestClock(Mutex<ClockFrame>);

impl TestClock {
    fn set(&self, (local_date, local_time): (Date, Time)) {
        *self.0.lock().unwrap() = ClockFrame {
            local_date,
            local_time,
            utc_offset: 0,
            daylight_savings_status: false,
        };
    }
}

impl ClockReader for TestClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(*self.0.lock().unwrap())
    }
}

/// What the test application was told.
#[derive(Debug, Clone, PartialEq)]
enum Told {
    Shed(ShedRequest),
    Stop,
}

/// An application whose answer the test sets, and which records what it is
/// told.
struct TestShedder {
    answer: Mutex<ShedStatus>,
    told: Mutex<Vec<Told>>,
}

impl TestShedder {
    fn new(progress: ShedProgress, expected: u64, actual: u64) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(ShedStatus::new(
                progress,
                BACnetShedLevel::Level(expected),
                BACnetShedLevel::Level(actual),
            )),
            told: Mutex::new(Vec::new()),
        })
    }

    fn answer(&self, progress: ShedProgress, expected: u64, actual: u64) {
        *self.answer.lock().unwrap() = ShedStatus::new(
            progress,
            BACnetShedLevel::Level(expected),
            BACnetShedLevel::Level(actual),
        );
    }

    fn answer_status(&self, status: ShedStatus) {
        *self.answer.lock().unwrap() = status;
    }

    fn take(&self) -> Vec<Told> {
        core::mem::take(&mut *self.told.lock().unwrap())
    }
}

impl LoadShedder for TestShedder {
    fn shed(&self, request: &ShedRequest) -> ShedStatus {
        self.told.lock().unwrap().push(Told::Shed(request.clone()));
        self.answer.lock().unwrap().clone()
    }

    fn stop(&self) {
        self.told.lock().unwrap().push(Told::Stop);
    }
}

/// LC-1 on a clock at 10:00, with level 2 requested for 30 minutes over a
/// 10-minute duty window, and Shed_Levels 1, 2 and 3.
fn object() -> (LoadControlObject, Arc<TestClock>) {
    let clock = Arc::new(TestClock(Mutex::new(ClockFrame {
        local_date: DAY,
        local_time: at(0).1,
        utc_offset: 0,
        daylight_savings_status: false,
    })));
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.bind_clocks(&Clocks::default().with_clock(Some(clock.clone() as Arc<dyn ClockReader>)));
    lc.set_shed_levels(vec![
        (1, "one".into()),
        (2, "two".into()),
        (3, "three".into()),
    ]);
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    lc.set_duty_window(10);
    (lc, clock)
}

fn level(lc: &LoadControlObject, p: P) -> PropertyValue {
    lc.read_property(p, None).unwrap()
}

fn shed(value: u64) -> PropertyValue {
    shed_level_value(&BACnetShedLevel::Level(value))
}

fn present_value(lc: &LoadControlObject) -> ShedState {
    let PropertyValue::Enumerated(raw) = lc.read_property(P::PRESENT_VALUE, None).unwrap() else {
        panic!("Present_Value is an enumeration");
    };
    assert_eq!(ShedState::from_raw(raw), lc.shed_state());
    lc.shed_state()
}

/// Start_Time as the object serves it.
fn start_time(lc: &LoadControlObject) -> (Date, Time) {
    let PropertyValue::List(items) = lc.read_property(P::START_TIME, None).unwrap() else {
        panic!("Start_Time is a Date and a Time");
    };
    let [PropertyValue::Date(date), PropertyValue::Time(time)] = items.as_slice() else {
        panic!("Start_Time is a Date and a Time");
    };
    (*date, *time)
}

fn write_start(lc: &mut LoadControlObject, start: (Date, Time)) {
    lc.write_property(
        P::START_TIME,
        None,
        PropertyValue::List(vec![
            PropertyValue::Date(start.0),
            PropertyValue::Time(start.1),
        ]),
        None,
    )
    .unwrap();
}

/// One pass of the server's operation task, at clock `minutes` past 10:00.
fn pass(lc: &mut LoadControlObject, clock: &TestClock, minutes: u8) -> bool {
    clock.set(at(minutes));
    lc.advance_monotonic_time(Duration::from_secs(u64::from(minutes) * 60))
}

/// The idle request: every request property back at its idle value.
fn assert_idle(lc: &LoadControlObject, agreed_duty_window: u64) {
    assert_eq!(present_value(lc), INACTIVE);
    assert_eq!(start_time(lc), UNSPECIFIED_DATETIME);
    for (p, value) in [
        (P::REQUESTED_SHED_LEVEL, shed(0)),
        (P::EXPECTED_SHED_LEVEL, shed(0)),
        (P::ACTUAL_SHED_LEVEL, shed(0)),
        (P::SHED_DURATION, PropertyValue::Unsigned(0)),
        (P::DUTY_WINDOW, PropertyValue::Unsigned(agreed_duty_window)),
    ] {
        assert_eq!(level(lc, p), value, "{p:?}");
    }
}

#[test]
fn without_an_application_a_request_waits_then_runs_non_compliant_to_its_end() {
    let (mut lc, clock) = object();
    // Parameters written while no request is pending are only kept.
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(level(&lc, P::SHED_DURATION), PropertyValue::Unsigned(30));
    // ReceiveShedRequest, then ReconfigurePending: Start_Time is ahead.
    write_start(&mut lc, at(5));
    assert_eq!(present_value(&lc), PENDING);
    assert_eq!(start_time(&lc), at(5));
    assert!(!pass(&mut lc, &clock, 4));
    assert_eq!(present_value(&lc), PENDING);
    // CannotMeetShed: nothing sheds, so the object can't comply.
    assert!(pass(&mut lc, &clock, 5));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert_eq!(level(&lc, P::EXPECTED_SHED_LEVEL), shed(0));
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(0));
    assert!(!pass(&mut lc, &clock, 34));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    // FinishedUnsuccessfulShed at Start_Time plus Shed_Duration.
    assert!(pass(&mut lc, &clock, 35));
    assert_idle(&lc, 0);
    assert!(!pass(&mut lc, &clock, 36));
}

#[test]
fn the_application_moves_a_shed_between_compliant_and_non_compliant() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Waiting, 2, 0);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(5));
    // Pending: the application's expectation is served.
    assert_eq!(present_value(&lc), PENDING);
    assert_eq!(level(&lc, P::EXPECTED_SHED_LEVEL), shed(2));
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(0));
    let told = app.take();
    assert_eq!(
        told,
        [Told::Shed(ShedRequest {
            requested: BACnetShedLevel::Level(2),
            shed_level: Some(1),
            start_time: at(5),
            shed_duration: 30,
            duty_window: 10,
            started: false,
        })]
    );
    // AbleToMeetShed at Start_Time.
    app.answer(ShedProgress::Compliant, 2, 2);
    assert!(pass(&mut lc, &clock, 5));
    assert_eq!(present_value(&lc), COMPLIANT);
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(2));
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.started));
    // CanNoLongerComplyWithShed, then CanNowComplyWithShed.
    app.answer(ShedProgress::Shedding, 2, 1);
    assert!(pass(&mut lc, &clock, 12));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(1));
    app.answer(ShedProgress::Compliant, 2, 2);
    assert!(pass(&mut lc, &clock, 20));
    assert_eq!(present_value(&lc), COMPLIANT);
    // A pass that changes nothing reports no change.
    assert!(!pass(&mut lc, &clock, 21));
    app.take();
    // FinishedSuccessfulShed: the application is told to stop, once.
    assert!(pass(&mut lc, &clock, 35));
    assert_idle(&lc, 0);
    assert_eq!(app.take(), [Told::Stop]);
    assert!(!pass(&mut lc, &clock, 40));
    assert!(app.take().is_empty());
}

#[test]
fn shedding_before_start_time_is_prepare_to_shed_or_early_compliance() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Shedding, 2, 1);
    lc.set_shedder(app.clone());
    // PrepareToShed: shedding begins ahead of Start_Time.
    write_start(&mut lc, at(15));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    // AbleToMeetShed's early form, through CanNowComplyWithShed here.
    app.answer(ShedProgress::Compliant, 2, 2);
    assert!(pass(&mut lc, &clock, 10));
    assert_eq!(present_value(&lc), COMPLIANT);
    // And straight from pending: Start_Time moved ahead again.
    let (mut lc, _clock) = object();
    lc.set_shedder(TestShedder::new(ShedProgress::Compliant, 2, 2));
    write_start(&mut lc, at(15));
    assert_eq!(present_value(&lc), COMPLIANT);
    assert_eq!(start_time(&lc), at(15));
}

#[test]
fn a_pending_request_is_cancelled_by_a_default_level_an_unspecified_start_or_its_end() {
    // What cancels a request for 10:15, with the clock at 10:10.
    type Cancel = fn(&mut LoadControlObject);
    let cancels: [(&str, Cancel); 3] = [
        ("default level", |lc| {
            lc.write_property(
                P::REQUESTED_SHED_LEVEL,
                None,
                shed_level_value(&BACnetShedLevel::Level(0)),
                None,
            )
            .unwrap();
        }),
        ("unspecified Start_Time", |lc| {
            write_start(lc, UNSPECIFIED_DATETIME);
        }),
        // 08:00 plus Shed_Duration 30 is over: a request for a time that has
        // passed.
        ("Start_Time long past", |lc| {
            write_start(lc, (DAY, Time { hour: 8, ..at(0).1 }));
        }),
    ];
    for (what, cancel) in cancels {
        let (mut lc, clock) = object();
        let app = TestShedder::new(ShedProgress::Waiting, 2, 0);
        lc.set_shedder(app.clone());
        lc.set_agreed_duty_window(15);
        write_start(&mut lc, at(15));
        assert_eq!(present_value(&lc), PENDING, "{what}");
        clock.set(at(10));
        app.take();
        cancel(&mut lc);
        assert_idle(&lc, 15);
        assert_eq!(app.take().last(), Some(&Told::Stop), "{what}");
    }
    // A Start_Time written when its request is already over starts nothing.
    let (mut lc, _clock) = object();
    write_start(&mut lc, (DAY, Time { hour: 9, ..at(0).1 }));
    assert_idle(&lc, 0);
}

#[test]
fn a_write_during_a_shed_reconfigures_it_through_pending() {
    for (progress, state) in [
        (ShedProgress::Compliant, COMPLIANT),
        (ShedProgress::Shedding, NON_COMPLIANT),
    ] {
        let (mut lc, clock) = object();
        let app = TestShedder::new(progress, 2, 2);
        lc.set_shedder(app.clone());
        write_start(&mut lc, at(0));
        assert_eq!(present_value(&lc), state);
        // Successful/UnsuccessfulShedReconfigured, then ReconfigurePending:
        // the new Start_Time is ahead and the application waits for it.
        clock.set(at(10));
        app.answer(ShedProgress::Waiting, 3, 0);
        app.take();
        write_start(&mut lc, at(20));
        assert_eq!(present_value(&lc), PENDING);
        assert!(matches!(&app.take()[..], [Told::Shed(r)] if !r.started && r.start_time == at(20)));
        // Each of the other three parameters reconfigures the same way: the
        // application is asked about the new request, and the state follows
        // its answer.
        app.answer(progress, 3, 3);
        assert!(pass(&mut lc, &clock, 20));
        assert_eq!(present_value(&lc), state);
        let writes = [
            (P::SHED_DURATION, PropertyValue::Unsigned(40)),
            (P::DUTY_WINDOW, PropertyValue::Unsigned(5)),
            (P::REQUESTED_SHED_LEVEL, shed(3)),
        ];
        for (p, value) in writes {
            app.take();
            lc.write_property(p, None, value, None).unwrap();
            assert_eq!(present_value(&lc), state, "{p:?}");
            assert!(
                matches!(&app.take()[..], [Told::Shed(r)] if r.started),
                "{p:?}"
            );
        }
        let last = ShedRequest {
            requested: BACnetShedLevel::Level(3),
            shed_level: Some(2),
            start_time: at(20),
            shed_duration: 40,
            duty_window: 5,
            started: true,
        };
        assert!(!pass(&mut lc, &clock, 21));
        assert_eq!(app.take(), [Told::Shed(last)]);
        // The reconfigured request ends at its new end, 10:20 plus 40.
        assert!(!pass(&mut lc, &clock, 59));
        assert!(pass(&mut lc, &clock, 60));
        assert_idle(&lc, 0);
    }
}

#[test]
fn a_shed_levels_write_asks_the_application_again_without_reconfiguring() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    clock.set(at(5));
    app.take();
    // Level 2 now selects the first element, the nearest lower value.
    app.answer(ShedProgress::Shedding, 2, 1);
    lc.write_property(
        P::SHED_LEVELS,
        None,
        PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(5),
            PropertyValue::Unsigned(9),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.shed_level == Some(0)));
}

#[test]
fn enable_false_ends_a_request_and_leaves_later_ones_unacted_on() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    app.take();
    lc.write_property(P::ENABLE, None, PropertyValue::Boolean(false), None)
        .unwrap();
    assert_idle(&lc, 0);
    assert_eq!(app.take(), [Told::Stop]);
    // A request written while disabled is kept, but nothing runs it.
    lc.set_requested_shed_level(BACnetShedLevel::Level(3))
        .unwrap();
    lc.set_shed_duration(20);
    write_start(&mut lc, at(10));
    assert!(!pass(&mut lc, &clock, 12));
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(start_time(&lc), at(10));
    assert_eq!(level(&lc, P::REQUESTED_SHED_LEVEL), shed(3));
    assert_eq!(level(&lc, P::EXPECTED_SHED_LEVEL), shed(0));
    // Enabling again starts nothing by itself; the next Start_Time does.
    lc.set_enable(true);
    assert!(!pass(&mut lc, &clock, 13));
    assert_eq!(present_value(&lc), INACTIVE);
    assert!(app.take().is_empty());
    write_start(&mut lc, at(14));
    assert_eq!(present_value(&lc), COMPLIANT);
    // Disabling an idle object changes nothing else.
    let (mut idle, _clock) = object();
    idle.set_enable(false);
    assert_eq!(level(&idle, P::SHED_DURATION), PropertyValue::Unsigned(30));
    assert_eq!(present_value(&idle), INACTIVE);
}

#[test]
fn without_a_clock_a_request_waits_but_still_cancels() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.set_requested_shed_level(BACnetShedLevel::Percent(80))
        .unwrap();
    lc.set_shed_duration(30);
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), PENDING);
    assert!(!lc.advance_monotonic_time(Duration::from_secs(3600)));
    assert_eq!(present_value(&lc), PENDING);
    // percent [0] 100, the PERCENT default, cancels.
    lc.set_requested_shed_level(BACnetShedLevel::Percent(100))
        .unwrap();
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(start_time(&lc), UNSPECIFIED_DATETIME);
}

#[test]
fn levels_the_application_reports_outside_the_requested_units_read_as_none_shed() {
    let (mut lc, _clock) = object();
    let app = TestShedder::new(ShedProgress::Waiting, 0, 0);
    lc.set_shedder(app.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Percent(70))
        .unwrap();
    for (expected, actual) in [
        (BACnetShedLevel::Level(2), BACnetShedLevel::Amount(1.0)),
        (
            BACnetShedLevel::Percent(101),
            BACnetShedLevel::Percent(u64::MAX),
        ),
    ] {
        app.answer_status(ShedStatus::new(ShedProgress::Waiting, expected, actual));
        write_start(&mut lc, at(15));
        assert_eq!(present_value(&lc), PENDING);
        for p in [P::EXPECTED_SHED_LEVEL, P::ACTUAL_SHED_LEVEL] {
            assert_eq!(
                level(&lc, p),
                shed_level_value(&BACnetShedLevel::Percent(100))
            );
        }
    }
    app.answer_status(ShedStatus::new(
        ShedProgress::Waiting,
        BACnetShedLevel::Percent(70),
        BACnetShedLevel::Percent(90),
    ));
    write_start(&mut lc, at(15));
    assert_eq!(
        level(&lc, P::ACTUAL_SHED_LEVEL),
        shed_level_value(&BACnetShedLevel::Percent(90))
    );
}

/// An application that panics on every call.
#[cfg(feature = "std")]
struct Panicking;

#[cfg(feature = "std")]
impl LoadShedder for Panicking {
    fn shed(&self, _: &ShedRequest) -> ShedStatus {
        panic!("the shedder failed");
    }

    fn stop(&self) {
        panic!("the shedder failed");
    }
}

// std: only std catches the application's panic.
#[cfg(feature = "std")]
#[test]
fn a_panicking_application_counts_as_nothing_shed() {
    let (mut lc, clock) = object();
    lc.set_shedder(Arc::new(Panicking));
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert!(pass(&mut lc, &clock, 30));
    assert_idle(&lc, 0);
}

#[test]
fn a_request_runs_across_midnight() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    let late = (
        DAY,
        Time {
            hour: 23,
            minute: 50,
            second: 0,
            hundredths: 0,
        },
    );
    let next_day = |minute: u8| {
        (
            Date {
                day: 12,
                day_of_week: 4,
                ..DAY
            },
            Time {
                hour: 0,
                minute,
                second: 0,
                hundredths: 0,
            },
        )
    };
    clock.set(late);
    write_start(&mut lc, late);
    assert_eq!(present_value(&lc), COMPLIANT);
    clock.set(next_day(19));
    assert!(!lc.advance_monotonic_time(Duration::ZERO));
    assert_eq!(present_value(&lc), COMPLIANT);
    clock.set(next_day(20));
    assert!(lc.advance_monotonic_time(Duration::ZERO));
    assert_idle(&lc, 0);
}

#[test]
fn the_agreed_duty_window_fills_in_for_one_not_written() {
    let (mut lc, clock) = object();
    // The agreed value shows at once while no request is pending.
    lc.set_agreed_duty_window(15);
    assert_eq!(level(&lc, P::DUTY_WINDOW), PropertyValue::Unsigned(15));
    // A Duty_Window written for this request holds through it.
    lc.set_duty_window(10);
    write_start(&mut lc, at(0));
    assert_eq!(level(&lc, P::DUTY_WINDOW), PropertyValue::Unsigned(10));
    assert!(pass(&mut lc, &clock, 30));
    assert_idle(&lc, 15);
    // The next request, with none written, takes the agreed one.
    let app = TestShedder::new(ShedProgress::Compliant, 1, 1);
    lc.set_shedder(app.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Level(1))
        .unwrap();
    lc.set_shed_duration(10);
    write_start(&mut lc, at(30));
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.duty_window == 15));
}

#[test]
fn a_cov_snapshot_reads_as_the_object() {
    let (mut lc, _clock) = object();
    lc.set_shedder(TestShedder::new(ShedProgress::Compliant, 2, 2));
    write_start(&mut lc, at(0));
    let snapshot = lc.cov_snapshot().unwrap();
    for row in lc.property_metadata().iter() {
        let p = row.property_identifier;
        assert_eq!(
            snapshot.read_property(p, None).unwrap(),
            level(&lc, p),
            "{p:?}"
        );
    }
}

#[test]
fn refused_writes_leave_the_request_alone() {
    let (mut lc, _clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(0));
    app.take();
    for (p, value) in [
        (P::START_TIME, PropertyValue::Null),
        (P::SHED_DURATION, PropertyValue::Real(1.0)),
        (P::DUTY_WINDOW, PropertyValue::Null),
        (P::REQUESTED_SHED_LEVEL, PropertyValue::Unsigned(1)),
        (P::ENABLE, PropertyValue::Unsigned(0)),
    ] {
        let error = lc.write_property(p, None, value, None).unwrap_err();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY && code == ErrorCode::INVALID_DATA_TYPE),
            "{p:?}: {error:?}"
        );
        assert_eq!(present_value(&lc), COMPLIANT);
    }
    assert!(app.take().is_empty());
}
