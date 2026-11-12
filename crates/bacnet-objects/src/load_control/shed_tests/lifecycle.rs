//! A request's life across clock edges, shedder changes and successive
//! requests on one object (#2464).

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// A Device clock that, once made flaky, answers every other read with no
/// frame, starting with a good one.
struct FlakyClock {
    frame: Mutex<ClockFrame>,
    flaky: AtomicBool,
    reads: AtomicU32,
}

impl ClockReader for FlakyClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        if self.flaky.load(Ordering::SeqCst) && self.reads.fetch_add(1, Ordering::SeqCst) % 2 == 1 {
            return None;
        }
        Some(*self.frame.lock().unwrap())
    }
}

/// A frame that names no actual date and time: the weekday disagrees.
fn unreadable(clock: &TestClock) {
    clock.set((
        Date {
            day_of_week: 1,
            ..DAY
        },
        at(10).1,
    ));
}

#[test]
fn a_clock_set_back_before_start_time_looks_at_the_request_again() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(5));
    // Compliant early, and still so once Start_Time has come.
    assert!(!pass(&mut lc, &clock, 5));
    assert_eq!(present_value(&lc), COMPLIANT);
    // The clock goes back to 10:02, before Start_Time. As though Start_Time
    // had been written: waiting loads put the request back to pending,
    // shedding ones keep it non-compliant ahead of time, and compliant ones
    // keep it compliant.
    for (progress, state) in [
        (ShedProgress::Waiting, PENDING),
        (ShedProgress::Shedding, NON_COMPLIANT),
        (ShedProgress::Compliant, COMPLIANT),
        (ShedProgress::Waiting, PENDING),
    ] {
        app.answer(progress, 2, 0);
        app.take();
        pass(&mut lc, &clock, 2);
        assert_eq!(present_value(&lc), state, "{progress:?}");
        assert!(matches!(&app.take()[..], [Told::Shed(r)] if !r.started));
    }
    // Back at Start_Time the request goes on.
    app.answer(ShedProgress::Compliant, 2, 2);
    assert!(pass(&mut lc, &clock, 5));
    assert_eq!(present_value(&lc), COMPLIANT);
}

#[test]
fn a_shed_holds_its_state_while_the_clock_cant_be_read() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Shedding, 2, 1);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    app.take();
    // A weekday that disagrees with the date: no actual date and time.
    clock.set((
        Date {
            day_of_week: 1,
            ..DAY
        },
        at(10).1,
    ));
    app.answer(ShedProgress::Waiting, 0, 0);
    assert!(!lc.advance_monotonic_time(Duration::ZERO));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(1));
    assert!(app.take().is_empty());
}

#[test]
fn a_clock_set_back_takes_a_non_compliant_shed_back_to_pending() {
    // Non-compliant after Start_Time, and ahead of it (PrepareToShed).
    for (start, first_look) in [(5, Some(5)), (10, None)] {
        let (mut lc, clock) = object();
        let app = TestShedder::new(ShedProgress::Shedding, 2, 1);
        lc.set_shedder(app.clone());
        write_start(&mut lc, at(start));
        if let Some(minutes) = first_look {
            pass(&mut lc, &clock, minutes);
        }
        assert_eq!(present_value(&lc), NON_COMPLIANT, "start {start}");
        // The clock goes back to 10:02 and the loads wait for Start_Time.
        app.answer(ShedProgress::Waiting, 2, 0);
        assert!(pass(&mut lc, &clock, 2));
        assert_eq!(present_value(&lc), PENDING, "start {start}");
    }
}

#[test]
fn one_look_reads_the_clock_once() {
    let clock = Arc::new(FlakyClock {
        frame: Mutex::new(ClockFrame {
            local_date: DAY,
            local_time: at(0).1,
            utc_offset: 0,
            daylight_savings_status: false,
        }),
        flaky: AtomicBool::new(false),
        reads: AtomicU32::new(0),
    });
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.bind_clocks(&Clocks::default().with_clock(Some(clock.clone() as Arc<dyn ClockReader>)));
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    clock.frame.lock().unwrap().local_time = at(5).1;
    write_start(&mut lc, at(5));
    assert_eq!(present_value(&lc), COMPLIANT);
    // Set back to 10:02, with the loads shedding short of the level. The
    // look's one good reading decides it: non-compliant ahead of Start_Time,
    // with the levels the application reports, and never a pending request
    // whose levels a second, failed reading reset.
    clock.frame.lock().unwrap().local_time = at(2).1;
    app.answer(ShedProgress::Shedding, 2, 1);
    clock.flaky.store(true, Ordering::SeqCst);
    assert!(lc.advance_monotonic_time(Duration::ZERO));
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(1));
}

#[test]
fn a_pending_request_holds_its_levels_while_the_clock_cant_be_read() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Waiting, 2, 1);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(5));
    assert_eq!(present_value(&lc), PENDING);
    // The clock comes and goes; nothing the client reads changes.
    for _ in 0..3 {
        unreadable(&clock);
        assert!(!lc.advance_monotonic_time(Duration::ZERO));
        assert_eq!(present_value(&lc), PENDING);
        assert_eq!(level(&lc, P::EXPECTED_SHED_LEVEL), shed(2));
        assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(1));
        assert!(!pass(&mut lc, &clock, 1));
    }
    // A new requested choice still puts them in its units.
    unreadable(&clock);
    lc.set_requested_shed_level(BACnetShedLevel::Percent(80))
        .unwrap();
    assert_eq!(present_value(&lc), PENDING);
    for p in [P::EXPECTED_SHED_LEVEL, P::ACTUAL_SHED_LEVEL] {
        assert_eq!(
            level(&lc, p),
            shed_level_value(&BACnetShedLevel::Percent(100))
        );
    }
}

#[test]
fn each_request_is_stopped_once_and_an_at_once_end_after_it_is_silent() {
    let (mut lc, clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    write_start(&mut lc, at(0));
    assert!(pass(&mut lc, &clock, 30));
    assert_eq!(app.take().last(), Some(&Told::Stop));
    // A Start_Time behind the default level, then one already over: each
    // ends its request at once, and the application hears nothing.
    lc.set_requested_shed_level(BACnetShedLevel::Level(0))
        .unwrap();
    write_start(&mut lc, at(31));
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    write_start(&mut lc, (DAY, Time { hour: 8, ..at(0).1 }));
    assert_eq!(present_value(&lc), INACTIVE);
    assert!(app.take().is_empty());
}

#[test]
fn a_replaced_shedder_hears_only_of_requests_it_was_asked_about() {
    let (mut lc, clock) = object();
    let first = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(first.clone());
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    first.take();
    // Replaced during the request, then the request is cancelled: neither
    // shedder is told, the first because it was replaced, the second
    // because it never saw the request.
    let second = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(second.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Level(0))
        .unwrap();
    assert_eq!(present_value(&lc), INACTIVE);
    assert!(first.take().is_empty());
    assert!(second.take().is_empty());
    // A request the second one is asked about is stopped through it.
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    write_start(&mut lc, at(1));
    assert!(pass(&mut lc, &clock, 31));
    assert_eq!(second.take().last(), Some(&Told::Stop));
    assert!(first.take().is_empty());
}

#[test]
fn the_shedder_reads_the_full_duty_baseline_and_hears_it_change() {
    let (mut lc, _clock) = object();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    // Idle: a new baseline is configuration, and nobody is asked.
    lc.set_full_duty_baseline(Some(400.0)).unwrap();
    assert!(app.take().is_empty());
    write_start(&mut lc, at(0));
    assert!(matches!(&app.take()[..],
        [Told::Shed(r)] if r.full_duty_baseline == Some(400.0)));
    // During a request the application is asked again with the new value,
    // and with none once it is cleared (#2479).
    lc.set_full_duty_baseline(Some(350.0)).unwrap();
    assert!(matches!(&app.take()[..],
        [Told::Shed(r)] if r.full_duty_baseline == Some(350.0)));
    lc.set_full_duty_baseline(None).unwrap();
    assert!(matches!(&app.take()[..],
        [Told::Shed(r)] if r.full_duty_baseline.is_none()));
    // A refused value changes nothing and asks nothing.
    assert!(lc.set_full_duty_baseline(Some(-5.0)).is_err());
    assert!(app.take().is_empty());
    assert_eq!(present_value(&lc), COMPLIANT);
}
