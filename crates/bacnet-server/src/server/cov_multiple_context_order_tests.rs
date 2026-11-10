//! COV-multiple contexts that report at once take their turns in key
//! order (#2320): by recipient, then subscriber process identifier, the
//! unconfirmed form before the confirmed one.
//!
//! One subscriber's contexts differ in their process identifier, so that is
//! the order it sees its reports in, whichever order it subscribed in. A
//! change that reports to several contexts starts that order from a place
//! that moves with every such fanout (the `budget` tests say why), so its
//! reports are a rotation of the key order; a follow-up pass, which gives
//! each context a budget of its own, starts from the first. The fanout
//! kept its contexts in a hash map before, and their order changed from
//! run to run. Time is paused.
use super::cov_wire_test_support::*;
use super::*;

/// The same contexts under a spent notification budget.
#[path = "cov_multiple_budget_tests.rs"]
mod budget;

/// Process identifiers in the order the contexts subscribe.
const SUBSCRIBED: [u32; 8] = [5, 1, 7, 3, 8, 2, 6, 4];

/// One unconfirmed context per process identifier, each with a timestamped
/// Present_Value reference to AV-1, their initial reports taken.
async fn contexts(delay: u32) -> Harness {
    let mut h = Harness::start(ServerConfig::default()).await;
    for process in SUBSCRIBED {
        h.subscribe_process(process, false, vec![(av1(), vec![(PV, true)])], Some(delay))
            .await;
        assert_eq!(
            h.notification().await.subscriber_process_identifier,
            process
        );
    }
    h
}

/// The process identifiers of the next reports, one per context.
async fn reported(h: &Harness) -> Vec<u32> {
    let mut processes = Vec::new();
    for _ in SUBSCRIBED {
        processes.push(h.notification().await.subscriber_process_identifier);
    }
    processes
}

#[tokio::test(start_paused = true)]
async fn one_change_reports_to_its_contexts_in_key_order_from_a_moving_start() {
    let mut h = contexts(10).await;
    h.set_clock(11);
    h.write_local(5.0).await;
    assert_eq!(reported(&h).await, [3, 4, 5, 6, 7, 8, 1, 2]);
    h.write_local(6.0).await;
    assert_eq!(reported(&h).await, [8, 1, 2, 3, 4, 5, 6, 7]);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn contexts_due_at_one_deadline_report_in_key_order() {
    let mut h = contexts(10).await;
    // Every send fails, so each context's change waits for its
    // Max_Notification_Delay, and all of them come due together.
    h.fail_notifications.store(true, Ordering::Release);
    h.set_clock(11);
    h.write_local(5.0).await;
    h.fail_notifications.store(false, Ordering::Release);
    // Nothing until just before the delay has passed.
    tokio::time::sleep(Duration::from_millis(9_900)).await;
    h.no_notification().await;
    assert_eq!(reported(&h).await, [1, 2, 3, 4, 5, 6, 7, 8]);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
