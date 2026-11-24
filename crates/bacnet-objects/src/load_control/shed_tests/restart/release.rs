//! A start that doesn't finish releases a restored shed (#2752): the
//! application is told to stop, the request stays in the object and in
//! storage, and the next start's look offers it again.

use super::*;

/// A request being shed, saved, and LC-1 rebuilt from it at 10:10 on
/// `clock`, with `app` installed when `with_app`.
fn restored_shed(
    store: &Arc<Store>,
    clock: &Arc<TestClock>,
    app: &Arc<TestShedder>,
    with_app: bool,
) -> LoadControlObject {
    let mut lc = kept(store, clock, app);
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    power_off(lc);
    app.take();
    clock.set(at(10));
    if with_app {
        return kept(store, clock, app);
    }
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    lc.set_shed_levels(vec![(1, "one".into()), (2, "two".into())])
        .unwrap();
    lc.bind_clocks(&Clocks::default().with_clock(Some(clock.clone() as Arc<dyn ClockReader>)));
    lc
}

#[test]
fn a_released_shed_stops_the_loads_and_keeps_the_request() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = restored_shed(&store, &clock, &app, true);
    lc.start_output();
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.started));
    lc.release_output();
    assert_eq!(app.take(), [Told::Stop]);
    // The request stays as the look left it, and storage with it.
    assert_eq!(present_value(&lc), COMPLIANT);
    assert_request(&lc, at(0));
    lc.wait_for_saves();
    assert_eq!(store.held().state, COMPLIANT);
    assert_eq!(store.held().start_time, at(0));
    // The next start offers it again, and its end stops the loads once.
    lc.start_output();
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.started));
    assert!(pass(&mut lc, &clock, 30));
    assert_eq!(app.take(), [Told::Stop]);
}

#[test]
fn without_a_shedder_a_release_leaves_the_stop_owed() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = restored_shed(&store, &clock, &app, false);
    lc.start_output();
    lc.release_output();
    assert!(app.take().is_empty());
    // A shedder installed later hears the end of the request the loads were
    // shed for before the restart, once.
    lc.set_shedder(app.clone());
    assert!(pass(&mut lc, &clock, 30));
    assert_eq!(app.take(), [Told::Stop]);
}
