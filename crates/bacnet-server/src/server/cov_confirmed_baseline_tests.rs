//! A confirmed COV report advances its subscriber's baseline only on the Ack
//! (#896).
//!
//! Every case runs for ordinary SubscribeCOV and for SubscribeCOVPropertyMultiple
//! on a Binary Value, whose Present_Value has no increment, and reads the decoded
//! wire. Time is paused, so each wait also lets the server finish the work it has
//! ready, such as handling an Ack and the fanout that follows it.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_device_core::cov::CovSample;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_types::enums::ObjectType;

const INACTIVE: u32 = 0;
const ACTIVE: u32 = 1;

#[derive(Clone, Copy, Debug)]
enum Family {
    Cov,
    Multiple,
}

const FAMILIES: [Family; 2] = [Family::Cov, Family::Multiple];

fn bv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap()
}

fn enumerated(value: u32) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut encoded,
        &PropertyValue::Enumerated(value),
    )
    .unwrap();
    encoded.to_vec()
}

async fn start(cov_retry_timeout_ms: u64) -> Harness {
    Harness::start_with(
        ServerConfig {
            cov_retry_timeout_ms,
            ..ServerConfig::default()
        },
        |db| {
            db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
                .unwrap();
        },
    )
    .await
}

impl Family {
    /// Subscribe to BV-1 with confirmed notifications; a repeat replaces it.
    async fn subscribe(self, h: &mut Harness) {
        match self {
            Family::Cov => {
                let mut body = BytesMut::new();
                SubscribeCOVRequest {
                    subscriber_process_identifier: 896,
                    monitored_object_identifier: bv1(),
                    issue_confirmed_notifications: Some(true),
                    lifetime: Some(300),
                }
                .encode(&mut body)
                .unwrap();
                h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
            }
            Family::Multiple => {
                h.subscribe_specs(true, vec![(bv1(), vec![(PV, false)])])
                    .await
            }
        }
    }

    /// Present_Value of the next report.
    async fn report(self, h: &Harness) -> Vec<u8> {
        let (object, value) = match self {
            Family::Cov => {
                let report = h.cov_notification().await;
                let value = report
                    .list_of_values
                    .into_iter()
                    .find(|value| value.property_identifier == PV)
                    .map(|value| value.value);
                (report.monitored_object_identifier, value)
            }
            Family::Multiple => {
                let report = h.notification().await;
                assert_eq!(report.list_of_cov_notifications.len(), 1);
                let item = &report.list_of_cov_notifications[0];
                let value = item
                    .list_of_values
                    .iter()
                    .find(|value| value.property_identifier == PV)
                    .map(|value| value.value.clone());
                (item.monitored_object_identifier, value)
            }
        };
        assert_eq!(object, bv1());
        value.expect("Present_Value reported")
    }

    /// Subscribe and acknowledge the initial INACTIVE report.
    async fn start(self, h: &mut Harness) {
        self.subscribe(h).await;
        assert_eq!(self.report(h).await, enumerated(INACTIVE), "{self:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(h).await, Some(sample(INACTIVE)), "{self:?}");
    }
}

fn sample(value: u32) -> CovSample {
    CovSample::new(&PropertyValue::Enumerated(value)).unwrap()
}

/// Present_Value baseline of BV-1's one subscription.
async fn baseline(h: &Harness) -> Option<CovSample> {
    let mut table = h.server.cov_table.write().await;
    let subscriptions = table.subscriptions_for(&bv1());
    assert_eq!(subscriptions.len(), 1);
    subscriptions[0]
        .last_notified_observation
        .as_ref()
        .map(|observation| observation.sample().clone())
}

/// Whether BV-1's one subscription may start a confirmed report now.
async fn idle(h: &Harness) -> bool {
    let mut table = h.server.cov_table.write().await;
    let sub = table.subscriptions_for(&bv1())[0].clone();
    match sub.key().multiple_context() {
        Some(context) => table.context_idle(context, std::slice::from_ref(&sub)),
        None => table.confirmed_idle(&sub),
    }
}

/// Write BV-1's Present_Value, fanning COV out even when it is unchanged.
async fn write(h: &Harness, value: u32) {
    h.server
        .write_local(
            &bv1(),
            PV,
            None,
            PropertyValue::Enumerated(value),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

/// Drop the recorded retries of one unanswered report, checking that only
/// its retries were sent.
fn discard_retries(h: &Harness, (invoke_id, service_choice): (u8, ConfirmedServiceChoice)) {
    let frames: Vec<_> = std::mem::take(&mut *h.frames.lock().unwrap())
        .into_iter()
        .filter(|frame| !matches!(frame, Apdu::SimpleAck(_)))
        .collect();
    assert_eq!(frames.len(), usize::from(DEFAULT_APDU_RETRIES));
    for frame in frames {
        assert!(
            matches!(&frame, Apdu::ConfirmedRequest(request)
                if request.invoke_id == invoke_id && request.service_choice == service_choice),
            "only retries of the unanswered report: {frame:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_report_exhausted_holds_off_then_is_reported_again() {
    for family in FAMILIES {
        let mut h = start(10).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        // No answer to any retry.
        let unanswered = h.take_confirmed();
        h.workers_idle().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        discard_retries(&h, unanswered);
        // Inside the hold-off an unchanged fanout sends nothing, and nothing
        // is scheduled for when it ends.
        assert!(!idle(&h).await, "{family:?} holds off");
        write(&h, ACTIVE).await;
        h.no_notification().await;
        assert!(idle(&h).await, "{family:?} hold-off over");
        // The next fanout of the unchanged object reports the lost change.
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.no_notification().await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_report_rejected_holds_off_then_is_reported_again() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        h.reject().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        // Not retried at once, nor by the next fanout inside the hold-off.
        write(&h, ACTIVE).await;
        h.no_notification().await;
        assert!(!idle(&h).await, "{family:?} holds off");
        // One full retry cycle: the 3 s timeout times four attempts.
        tokio::time::sleep(Duration::from_secs(12)).await;
        assert!(idle(&h).await, "{family:?} hold-off over");
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.server.stop().await.unwrap();
    }
}

/// The subscriber serves a confirmed report, so its Abort has the server flag
/// set (Clause 5.4) and ends the report like an Error, with no retry. One with
/// the flag clear is refused and leaves the report outstanding (#1155).
#[tokio::test(start_paused = true)]
async fn confirmed_report_aborted_by_the_subscriber_is_not_retried() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        let (invoke_id, _) = h.take_confirmed();
        for sent_by_server in [false, true] {
            h.respond(Apdu::Abort(AbortPdu {
                sent_by_server,
                invoke_id,
                abort_reason: AbortReason::OTHER,
            }))
            .await;
            h.settle().await;
            assert_eq!(
                h.server.notification_transactions.active_count(),
                usize::from(!sent_by_server),
                "{family:?}: Abort with server flag {sent_by_server}"
            );
        }
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        // One full retry cycle, the 3 s timeout times four attempts, sends nothing.
        tokio::time::sleep(Duration::from_secs(12)).await;
        assert!(
            !h.frames
                .lock()
                .unwrap()
                .iter()
                .any(|frame| matches!(frame, Apdu::ConfirmedRequest(_))),
            "{family:?}: no retry after the Abort"
        );
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_follow_up_under_dcc_is_dropped_not_deferred() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        write(&h, INACTIVE).await;
        // DISABLE_INITIATION: the Ack still completes the baseline, but the
        // follow-up is suppressed like any fanout.
        h.server
            .comm_state
            .set_for_test(DccState::DisableInitiation);
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.no_notification().await;
        // Re-enabled: the dropped follow-up does not come back by itself.
        h.server.comm_state.set_for_test(DccState::Enable);
        h.no_notification().await;
        write(&h, INACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(INACTIVE), "{family:?}");
        h.server.stop().await.unwrap();
    }
}

/// DISABLE_INITIATION taking effect while a report is outstanding ends it at
/// its first retry, which is not sent, and frees its invoke ID there (Clause
/// 16.1, #1327). The subscriber did not fail, so no hold-off follows: the
/// change is owed as one DCC held back before its first send, and the first
/// fanout once initiation is enabled again reports it.
#[tokio::test(start_paused = true)]
async fn confirmed_report_dcc_stops_at_a_retry_ends_there_without_a_hold_off() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        let sent = tokio::time::Instant::now();
        let (invoke_id, _) = h.take_confirmed();
        h.server
            .comm_state
            .set_for_test(DccState::DisableInitiation);
        h.workers_idle().await;
        let ended = sent.elapsed();
        assert!(
            (Duration::from_millis(2_900)..Duration::from_millis(3_050)).contains(&ended),
            "{family:?}: lease freed {ended:?} after the report"
        );
        assert!(
            !h.frames.lock().unwrap().iter().any(
                |apdu| matches!(apdu, Apdu::ConfirmedRequest(request) if request.invoke_id == invoke_id)
            ),
            "{family:?}: no retry"
        );
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        assert!(idle(&h).await, "{family:?}: no hold-off");
        // While initiation stays restricted a fanout sends nothing, and
        // enabling it sends nothing by itself.
        write(&h, ACTIVE).await;
        h.no_notification().await;
        h.server.comm_state.set_for_test(DccState::Enable);
        h.no_notification().await;
        // The next fanout reports the change at once.
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_fanout_during_the_flight_sends_no_second_report() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        // The baseline is still INACTIVE, yet the outstanding report holds
        // this fanout back.
        write(&h, ACTIVE).await;
        h.no_notification().await;
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_change_during_the_flight_follows_the_ack() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        write(&h, INACTIVE).await;
        h.no_notification().await;
        // The Ack sends the current value with no further write.
        h.ack().await;
        assert_eq!(family.report(&h).await, enumerated(INACTIVE), "{family:?}");
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        h.no_notification().await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_ack_without_a_change_sends_nothing_more() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.no_notification().await;
        write(&h, ACTIVE).await;
        h.no_notification().await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_ack_for_a_replaced_subscription_leaves_the_new_one_alone() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.subscribe(&mut h).await;
        assert_eq!(family.report(&h).await, enumerated(INACTIVE), "{family:?}");
        let old = h.take_confirmed();
        write(&h, ACTIVE).await;
        // Renewal replaces the subscription; its initial report is its own.
        family.subscribe(&mut h).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        let new = h.take_confirmed();
        h.ack_request(old).await;
        h.settle().await;
        assert_eq!(baseline(&h).await, None, "{family:?}");
        // The replacement's report is still outstanding: this change waits.
        write(&h, INACTIVE).await;
        h.no_notification().await;
        h.ack_request(new).await;
        assert_eq!(family.report(&h).await, enumerated(INACTIVE), "{family:?}");
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(baseline(&h).await, Some(sample(INACTIVE)), "{family:?}");
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_superseded_report_stops_retrying() {
    for family in FAMILIES {
        let mut h = start(3000).await;
        family.start(&mut h).await;
        write(&h, ACTIVE).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        let (old, _) = h.take_confirmed();
        // A renewal or re-subscription replaces the subscription while its
        // report is outstanding; the replacement reports for itself.
        family.subscribe(&mut h).await;
        assert_eq!(family.report(&h).await, enumerated(ACTIVE), "{family:?}");
        h.ack().await;
        h.settle().await;
        assert_eq!(
            h.server.notification_transactions.active_count(),
            0,
            "{family:?}: the superseded transaction ended"
        );
        // Past the old report's retry time, none of its retries went out.
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(
            !h.frames.lock().unwrap().iter().any(
                |apdu| matches!(apdu, Apdu::ConfirmedRequest(request) if request.invoke_id == old)
            ),
            "{family:?}"
        );
        assert_eq!(baseline(&h).await, Some(sample(ACTIVE)), "{family:?}");
        h.server.stop().await.unwrap();
    }
}
