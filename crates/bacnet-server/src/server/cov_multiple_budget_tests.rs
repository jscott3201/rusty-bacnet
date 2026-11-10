//! COV-multiple contexts under a spent notification budget (#2320).
//!
//! One change may send only so many notifications
//! (`CovPolicy::max_notifications_per_event`). A context whose turn comes
//! after the budget is spent sends nothing: what its report drained goes
//! back to its references, and it takes its next turn ahead of the
//! contexts that were served, so the same ones don't wait every time. Time
//! is paused.
use super::super::cov_wire_test_support::*;
use super::super::*;

const DELAY: u32 = 10;

/// A server whose changes may each send one notification, with one
/// unconfirmed context per process identifier, 1 to 3, on AV-1's
/// Present_Value, their initial reports taken.
async fn contexts(timestamped: bool) -> Harness {
    let config = ServerConfig {
        cov_policy: bacnet_device_core::cov::CovPolicy {
            max_notifications_per_event: 1,
            ..Default::default()
        },
        ..ServerConfig::default()
    };
    let mut h = Harness::start(config).await;
    for process in 1..=3 {
        let specs = vec![(av1(), vec![(PV, timestamped)])];
        h.subscribe_process(process, false, specs, Some(DELAY))
            .await;
        let initial = h.notification().await;
        assert_eq!(initial.subscriber_process_identifier, process);
    }
    h
}

#[tokio::test(start_paused = true)]
async fn an_untimestamped_context_behind_a_spent_budget_has_the_next_turn() {
    let mut h = contexts(false).await;
    // Each change reports to one context. An untimestamped reference has
    // no history and no deadline, so a context that always came last
    // would never report: the ones kept back go first instead, the one
    // waiting longest ahead.
    let mut served = Vec::new();
    for change in 1..=6u8 {
        h.write_local(f32::from(change)).await;
        let report = h.notification().await;
        assert_eq!(pv_rows(&report), vec![(real(f32::from(change)), None)]);
        served.push(report.subscriber_process_identifier);
        h.no_notification().await;
    }
    assert_eq!(served, [1, 2, 3, 1, 2, 3]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn what_a_spent_budget_kept_back_is_in_the_next_report() {
    let mut h = contexts(true).await;
    h.set_clock(11);
    h.write_local(5.0).await;
    let first = h.notification().await;
    assert_eq!(first.subscriber_process_identifier, 1);
    assert_eq!(pv_rows(&first), vec![(real(5.0), Some(time(11)))]);
    h.no_notification().await;

    // The other two drained the change into their reports before the
    // budget stopped them. It is back in their histories: the next change
    // reports to the one waiting longest, with both changes.
    h.set_clock(12);
    h.write_local(6.0).await;
    let second = h.notification().await;
    assert_eq!(second.subscriber_process_identifier, 2);
    assert_eq!(
        pv_rows(&second),
        vec![(real(5.0), Some(time(11))), (real(6.0), Some(time(12)))]
    );
    h.no_notification().await;

    // The third context's two changes go out at their delay, under a
    // budget of the follow-up's own, and nothing more is pending.
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY))).await;
    let mut late = [h.notification().await, h.notification().await];
    late.sort_by_key(|report| report.subscriber_process_identifier);
    assert_eq!(late[0].subscriber_process_identifier, 1);
    assert_eq!(pv_rows(&late[0]), vec![(real(6.0), Some(time(12)))]);
    assert_eq!(late[1].subscriber_process_identifier, 3);
    assert_eq!(
        pv_rows(&late[1]),
        vec![(real(5.0), Some(time(11))), (real(6.0), Some(time(12)))]
    );
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
