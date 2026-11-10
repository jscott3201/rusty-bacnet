//! COV-multiple contexts under a spent notification budget (#2320).
//!
//! One change may send only so many notifications and octets
//! (`CovPolicy::max_notifications_per_event`,
//! `max_notification_bytes_per_event`). A context whose turn comes after
//! the budget is spent sends nothing: what its report drained goes back to
//! its references. The contexts take their turns in key order from a start
//! that moves with every fanout, so the ones kept back differ from change
//! to change and no context loses every time, whichever objects change.
//! The start comes from a fixed sequence in the core, so the turns here
//! are the same in every run. Time is paused.
use super::super::cov_wire_test_support::*;
use super::super::*;
use bacnet_device_core::cov::CovPolicy;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::enums::EngineeringUnits;

const DELAY: u32 = 10;

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

/// A server under `policy` with AV-1 and AV-2, and one unconfirmed context
/// per process identifier, 1 to `contexts`, on the Present_Value of both,
/// their initial reports taken.
async fn contexts(policy: CovPolicy, contexts: u32, timestamped: bool) -> Harness {
    let config = ServerConfig {
        cov_policy: policy,
        ..ServerConfig::default()
    };
    let mut h = Harness::start_with(config, |db| {
        let second = AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS);
        db.add(Box::new(second.unwrap())).unwrap();
    })
    .await;
    for process in 1..=contexts {
        // One request per object, so each initial report is one
        // notification whatever the budget.
        for object in [av(1), av(2)] {
            let specs = vec![(object, vec![(PV, timestamped)])];
            h.subscribe_process(process, false, specs, Some(DELAY))
                .await;
            h.notification().await;
        }
    }
    h
}

fn notifications(max: usize) -> CovPolicy {
    CovPolicy {
        max_notifications_per_event: max,
        ..CovPolicy::default()
    }
}

/// Every COV-multiple notification sent since the last call, in order: the
/// subscriber's process and the Analog Value it reports.
async fn sent(h: &Harness) -> Vec<(u32, u32)> {
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut all = Vec::new();
    h.frames.lock().unwrap().retain(|apdu| {
        let Apdu::UnconfirmedRequest(request) = apdu else {
            return true;
        };
        let report = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
        let object = report.list_of_cov_notifications[0].monitored_object_identifier;
        all.push((
            report.subscriber_process_identifier,
            object.instance_number(),
        ));
        false
    });
    all
}

/// Change AV-1 `changes` times; the processes each change reported to.
async fn run(h: &Harness, changes: u8) -> Vec<Vec<u32>> {
    let mut served = Vec::new();
    for change in 1..=changes {
        h.write_local(f32::from(change)).await;
        let reports = sent(h).await;
        served.push(reports.into_iter().map(|(process, _)| process).collect());
    }
    served
}

#[tokio::test(start_paused = true)]
async fn a_budget_of_one_serves_every_context_in_a_run() {
    let mut h = contexts(notifications(1), 3, false).await;
    let served = run(&h, 20).await.concat();
    // One notification a change, to the context the start falls on: all
    // three within the first three changes here, and none waits more than
    // ten changes over the sequence's first sixty.
    assert_eq!(
        served,
        [1, 3, 2, 1, 3, 3, 3, 3, 2, 3, 1, 3, 2, 1, 1, 1, 1, 2, 1, 3]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn three_contexts_on_two_objects_changing_in_turn_all_hear_of_both() {
    // Two notifications a change for three contexts, the two objects
    // changing in alternation: a start that came round in step with the
    // changes would keep one context from one object for good.
    let mut h = contexts(notifications(2), 3, false).await;
    let mut heard = std::collections::BTreeMap::new();
    for change in 0..60u8 {
        let object = u32::from(change % 2) + 1;
        h.write_local_to(av(object), f32::from(change) + 1.0).await;
        let reports = sent(&h).await;
        assert_eq!(reports.len(), 2, "change {change}");
        for report in reports {
            assert_eq!(report.1, object);
            *heard.entry(report).or_insert(0) += 1;
        }
    }
    let expected = [
        ((1, 1), 23),
        ((1, 2), 19),
        ((2, 1), 17),
        ((2, 2), 19),
        ((3, 1), 20),
        ((3, 2), 22),
    ];
    assert_eq!(heard.into_iter().collect::<Vec<_>>(), expected);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_octet_budget_serves_every_context_in_a_run() {
    // Each notification here takes 37 octets: a budget of 50 fits one, and
    // one of 100 two. The octet budget refuses without ever counting as
    // spent, and the moving start shares that out the same way.
    for (octets, each) in [(50, 1), (100, 2)] {
        let policy = CovPolicy {
            max_notification_bytes_per_event: octets,
            ..CovPolicy::default()
        };
        let mut h = contexts(policy, 3, false).await;
        let served = run(&h, 12).await;
        assert!(
            served.iter().all(|change| change.len() == each),
            "{served:?}"
        );
        let firsts: Vec<_> = served.iter().map(|change| change[0]).collect();
        assert_eq!(
            firsts,
            [1, 3, 2, 1, 3, 3, 3, 3, 2, 3, 1, 3],
            "{octets} octets"
        );
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn an_ample_budget_serves_every_context_every_time() {
    let mut h = contexts(CovPolicy::default(), 3, false).await;
    let served = run(&h, 6).await;
    // Key order from the moving start: every context, each time.
    let rotations = [
        [1, 2, 3],
        [3, 1, 2],
        [2, 3, 1],
        [1, 2, 3],
        [3, 1, 2],
        [3, 1, 2],
    ];
    assert_eq!(served, rotations);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn what_a_spent_budget_kept_back_is_in_the_next_report() {
    let mut h = contexts(notifications(1), 3, true).await;
    h.set_clock(11);
    h.write_local(5.0).await;
    let first = h.notification().await;
    assert_eq!(first.subscriber_process_identifier, 1);
    assert_eq!(pv_rows(&first), vec![(real(5.0), Some(time(11)))]);
    h.no_notification().await;

    // The other two drained the change into their reports before the
    // budget stopped them. It is back in their histories: the next change
    // reports to the third context, with both changes.
    h.set_clock(12);
    h.write_local(6.0).await;
    let second = h.notification().await;
    assert_eq!(second.subscriber_process_identifier, 3);
    let both = vec![(real(5.0), Some(time(11))), (real(6.0), Some(time(12)))];
    assert_eq!(pv_rows(&second), both);
    h.no_notification().await;

    // The second context's two changes and the first's newer one go out
    // at their delays, each under a budget of the follow-up's own, and
    // nothing more is pending.
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY))).await;
    let mut late = [h.notification().await, h.notification().await];
    late.sort_by_key(|report| report.subscriber_process_identifier);
    assert_eq!(late[0].subscriber_process_identifier, 1);
    assert_eq!(pv_rows(&late[0]), vec![(real(6.0), Some(time(12)))]);
    assert_eq!(late[1].subscriber_process_identifier, 2);
    assert_eq!(pv_rows(&late[1]), both);
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
