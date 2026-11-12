//! A save nobody waited for that failed is tried again (#2502): at the
//! next look once the writer is idle, then backing off from 2 s, doubling
//! to 64 s apart, until one succeeds; a change saves at once and starts
//! the schedule again. Time is set by hand on the looks.

use super::*;

fn attempts(store: &Store) -> u32 {
    store.attempts.load(Ordering::SeqCst)
}

/// A look at monotonic second `second`, as the once-a-second pass makes
/// one, then the wait for what it queued.
fn look_at(lc: &mut LoadControlObject, second: u64) {
    lc.advance_monotonic_time(Duration::from_secs(second));
    lc.wait_for_saves();
}

/// LC-1 kept in `store`, idle, its first saves landed, then a change whose
/// save fails.
fn failed(store: &Arc<Store>) -> LoadControlObject {
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(store, &test_clock(), &app);
    lc.wait_for_saves();
    store.fail.store(true, Ordering::SeqCst);
    lc.set_agreed_duty_window(5);
    lc.wait_for_saves();
    lc
}

#[test]
fn tries_back_off_from_two_seconds_to_sixty_four() {
    let store = Arc::new(Store::default());
    let mut lc = failed(&store);
    let before = attempts(&store);
    // The first try at the next look, whatever its time.
    look_at(&mut lc, 100);
    assert_eq!(attempts(&store), before + 1);
    // Then 2 s, 4, 8, 16, 32 and 64 apart, and 64 from then on.
    let mut due = 100;
    for (wait, tries) in [2, 4, 8, 16, 32, 64, 64, 64].into_iter().zip(2..) {
        look_at(&mut lc, due + wait - 1);
        assert_eq!(attempts(&store), before + tries - 1, "early, {wait} s");
        due += wait;
        look_at(&mut lc, due);
        assert_eq!(attempts(&store), before + tries, "due, {wait} s");
    }
    assert_eq!(store.held().agreed_duty_window, 0, "nothing landed");
}

#[test]
fn a_try_that_succeeds_ends_the_tries() {
    let store = Arc::new(Store::default());
    let mut lc = failed(&store);
    look_at(&mut lc, 10);
    look_at(&mut lc, 12);
    store.fail.store(false, Ordering::SeqCst);
    let before = attempts(&store);
    look_at(&mut lc, 16);
    assert_eq!(attempts(&store), before + 1);
    assert_eq!(store.held().agreed_duty_window, 5);
    // No save more after it, at any time.
    for second in [17, 20, 32, 100, 1000] {
        look_at(&mut lc, second);
    }
    assert_eq!(attempts(&store), before + 1);
}

#[test]
fn a_change_saves_at_once_and_starts_the_schedule_again() {
    let store = Arc::new(Store::default());
    let mut lc = failed(&store);
    for second in [10, 12, 16, 24] {
        look_at(&mut lc, second);
    }
    // Backed off to 16 s: the next try isn't due before 40.
    let before = attempts(&store);
    look_at(&mut lc, 30);
    assert_eq!(attempts(&store), before);
    lc.set_agreed_duty_window(6);
    lc.wait_for_saves();
    assert_eq!(attempts(&store), before + 1, "the change saved at once");
    // Its save failed too: tried at the next look, then 2 s on.
    look_at(&mut lc, 31);
    assert_eq!(attempts(&store), before + 2);
    look_at(&mut lc, 32);
    assert_eq!(attempts(&store), before + 2);
    look_at(&mut lc, 33);
    assert_eq!(attempts(&store), before + 3);
}

#[test]
fn an_older_failure_does_not_spend_the_try_a_newer_change_needs() {
    let store = Arc::new(Store::default());
    // A fails.
    let mut lc = failed(&store);
    let before = attempts(&store);
    // B blocks in storage, and C queues behind it.
    let (started, go) = {
        let (started, started_rx) = std::sync::mpsc::channel();
        let (go, go_rx) = std::sync::mpsc::channel();
        *store.hold.lock().unwrap() = Some((started, go_rx));
        (started_rx, go)
    };
    lc.set_agreed_duty_window(6);
    started
        .recv_timeout(Duration::from_secs(10))
        .expect("B reached storage");
    lc.set_agreed_duty_window(7);
    // A look while they are queued tries nothing: A's failure is old news.
    lc.advance_monotonic_time(Duration::from_secs(10));
    // B and C fail; then storage comes back.
    drop(go);
    lc.wait_for_saves();
    assert_eq!(attempts(&store), before + 2);
    store.fail.store(false, Ordering::SeqCst);
    look_at(&mut lc, 11);
    assert_eq!(attempts(&store), before + 3);
    assert_eq!(store.held().agreed_duty_window, 7, "C landed");
}
