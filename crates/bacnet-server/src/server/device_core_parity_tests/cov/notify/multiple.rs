//! SubscribeCOVPropertyMultiple and its notifications (#2320): the server
//! and the `Reporter` send the same reports, in the unconfirmed and the
//! confirmed form, for a timestamped reference with history, an
//! untimestamped one, a report split across APDUs, the
//! Max_Notification_Delay backstop, a failed confirmed report with its
//! follow-up, and a cancellation with changes pending.
//!
//! Both databases read one fixed Device clock, so every Time_Of_Change and
//! envelope timestamp agrees. A send that fails on the server is reported
//! unsent to the `Reporter`: the server's link refuses its next send, and
//! the `Reporter`'s first notification of the step comes back as not sent.
//! Contexts report in key order on both sides, so that first notification
//! is the same one, which the test checks too.
use super::*;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;

const MULTIPLE: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
/// Max_Notification_Delay of every context here, in seconds.
const DELAY: u32 = 10;

fn msv() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_VALUE, 1)
}

/// A SubscribeCOVPropertyMultiple body for `process`: Present_Value of each
/// of `references`, timestamped or not, for `timing`'s lifetime and
/// Max_Notification_Delay; a cancellation without `timing`.
fn multiple(
    process: u32,
    confirmed: bool,
    references: &[(ObjectIdentifier, bool)],
    timing: Option<(u32, u32)>,
) -> Vec<u8> {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: process,
        issue_confirmed_notifications: confirmed,
        lifetime: timing.map(|(lifetime, _)| lifetime),
        max_notification_delay: timing.map(|(_, delay)| delay),
        list_of_cov_subscription_specifications: references
            .iter()
            .map(|&(object, timestamped)| COVSubscriptionSpecification {
                monitored_object_identifier: object,
                list_of_cov_references: vec![COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PV,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped,
                }],
            })
            .collect(),
    }
    .encode(&mut body)
    .unwrap();
    body.to_vec()
}

impl CovParity {
    /// Both sides under the default policy, the server's database reading
    /// the fixed clock the core's reads.
    async fn start_multiple() -> Self {
        Self::start_multiple_under(CovPolicy::default()).await
    }

    /// [`Self::start_multiple`] under `policy`.
    async fn start_multiple_under(policy: CovPolicy) -> Self {
        let cov = Self::start(policy).await;
        // Start installs the system clock; serve the fixed one.
        cov.parity
            .server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        cov
    }

    /// The device changes AI-1's Present_Value itself on both sides. With
    /// `fail`, the first notification it calls for is not sent on either:
    /// the server's link refuses it, and the `Reporter` is told so. How
    /// many notifications both sent.
    async fn sample(&mut self, what: &str, value: f32, fail: bool) -> usize {
        let now = runtime_clock::core_now();
        if fail {
            self.link.fail_next_send();
        }
        let value = PropertyValue::Real(value);
        let source = crate::LocalCommandSource::ServerDevice;
        self.parity
            .server
            .write_local(&ai(), PV, None, value.clone(), None, source)
            .await
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut frames = self.parity.sent.take();
        let parity = &mut self.parity;
        parity
            .db
            .get_mut(&ai())
            .unwrap()
            .write_property(PV, None, value, None)
            .unwrap();
        parity.reporter.changed(&parity.db, &[ai()], now);
        if fail {
            let mut out = BytesMut::new();
            let unsent = parity
                .reporter
                .pop(&NoBindings, &mut self.ids, now, &mut out)
                .unwrap_or_else(|| panic!("{what}: the core reports nothing"));
            parity
                .reporter
                .delivered(unsent.token, Delivery::NotSent, &mut self.ids, now);
            // The link records a send before it refuses it: the two
            // notifications that were not sent are the same one.
            assert!(!frames.is_empty(), "{what}: the server reports nothing");
            let refused = frames.remove(0).decode_npdu().payload;
            assert_eq!(refused, out, "{what}: the unsent one, server then core");
        }
        self.same_notifications(what, &frames, now)
    }
}

#[tokio::test(start_paused = true)]
async fn unconfirmed_multiple_reports_match() {
    let mut cov = CovParity::start_multiple().await;
    let a = direct(1);
    let real = PropertyValue::Real;
    let timing = Some((300, DELAY));
    let timed = multiple(1, false, &[(ai(), true)], timing);
    let (reply, initial) = cov.exchange("timestamped", &a, MULTIPLE, &timed).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(initial, 1, "the initial report");
    cov.same_tables("subscribed").await;
    cov.read_list("listed", P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS)
        .await;
    let oos = PropertyValue::Boolean(true);
    assert_eq!(cov.write("Status_Flags", &a, ai(), OOS, oos).await, 1);
    assert_eq!(cov.write("a change", &a, ai(), PV, real(30.0)).await, 1);
    assert_eq!(cov.write("no change", &a, ai(), PV, real(30.0)).await, 0);

    // History: a change whose send fails stays queued, and the next
    // report carries it before the newer one.
    assert_eq!(cov.sample("a failed send", 31.0, true).await, 0);
    assert_eq!(cov.sample("with history", 32.0, false).await, 1);

    // The backstop: after a failed send and no further change, the report
    // goes out when its Max_Notification_Delay has passed.
    assert_eq!(cov.sample("failed again", 33.0, true).await, 0);
    let delay = u64::from(DELAY);
    assert_eq!(cov.advance("before the delay", delay - 1).await, 0);
    assert_eq!(cov.advance("at the delay", 2).await, 1);
    assert_eq!(cov.advance("delivered", 3 * delay).await, 0);

    // An untimestamped reference to the same value, in a context of its
    // own: one change reports to both, in key order.
    let untimed = multiple(2, false, &[(ai(), false)], timing);
    let (_, initial) = cov.exchange("untimestamped", &a, MULTIPLE, &untimed).await;
    assert_eq!(initial, 1);
    assert_eq!(cov.sample("both contexts", 34.0, false).await, 2);

    // A report split across APDUs: two objects' items don't fit one
    // notification to a 50-octet subscriber.
    cov.max_apdu = 50;
    let small = multiple(3, false, &[(ai(), false), (msv(), false)], timing);
    let (_, initial) = cov.exchange("split", &a, MULTIPLE, &small).await;
    assert_eq!(initial, 2, "one notification per object");
    cov.max_apdu = 1476;
    cov.read_list("three contexts", P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS)
        .await;

    // A cancellation with a change pending: the first context's send
    // fails, the other two go out, and the cancelled context's change is
    // dropped with it.
    assert_eq!(cov.sample("pending", 35.0, true).await, 2);
    let cancel = multiple(1, false, &[], None);
    let (reply, after) = cov.exchange("cancelled", &a, MULTIPLE, &cancel).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(after, 0);
    cov.same_tables("cancelled").await;
    assert_eq!(cov.advance("past the delay", delay + 2).await, 0);
    cov.read_list("two contexts", P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS)
        .await;
    cov.stop().await;
}

#[tokio::test(start_paused = true)]
async fn confirmed_multiple_reports_match() {
    let mut cov = CovParity::start_multiple().await;
    let a = direct(1);
    let real = PropertyValue::Real;
    let timing = Some((600, DELAY));
    // A timestamped reference and an untimestamped one in one context.
    let both = multiple(1, true, &[(ai(), true), (msv(), false)], timing);
    let (reply, initial) = cov.exchange("confirmed", &a, MULTIPLE, &both).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(initial, 1, "the initial report");
    cov.same_tables("subscribed").await;

    // History: changes made while the report is out wait for its Ack, and
    // its follow-up carries them all.
    let oos = PropertyValue::Boolean(true);
    assert_eq!(cov.write("held", &a, ai(), OOS, oos).await, 0);
    assert_eq!(cov.write("held too", &a, ai(), PV, real(30.0)).await, 0);
    assert_eq!(cov.answer("Ack, then the history", false).await, 1);
    assert_eq!(cov.answer("Ack", false).await, 0);

    // A refused report: its change, and one made in the hold-off, go out
    // in one follow-up when the hold-off ends.
    assert_eq!(cov.write("to refuse", &a, ai(), PV, real(40.0)).await, 1);
    assert_eq!(cov.answer("refused", true).await, 0);
    assert_eq!(cov.write("held off", &a, ai(), PV, real(50.0)).await, 0);
    assert_eq!(cov.advance("in the hold-off", 11).await, 0);
    assert_eq!(cov.advance("the hold-off's end", 2).await, 1);
    assert_eq!(cov.answer("Ack of the follow-up", false).await, 0);

    // A report split across APDUs goes out one part per Ack.
    cov.max_apdu = 50;
    let small = multiple(2, true, &[(ai(), false), (msv(), false)], timing);
    let (_, initial) = cov.exchange("split", &a, MULTIPLE, &small).await;
    assert_eq!(initial, 1, "the first part");
    cov.max_apdu = 1476;
    assert_eq!(cov.answer("Ack, then the second part", false).await, 1);
    assert_eq!(cov.answer("Ack of the second part", false).await, 0);
    cov.read_list("listed", P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS)
        .await;

    // A cancellation with changes pending: a refused report's change and
    // one more wait out the hold-off, and the cancellation drops both.
    assert_eq!(
        cov.write("both contexts", &a, ai(), PV, real(60.0)).await,
        2
    );
    assert_eq!(cov.answer("the first refuses", true).await, 0);
    assert_eq!(cov.answer("the second acknowledges", false).await, 0);
    assert_eq!(cov.write("pending", &a, ai(), PV, real(70.0)).await, 1);
    assert_eq!(cov.answer("Ack", false).await, 0);
    let cancel = multiple(1, true, &[], None);
    let (reply, after) = cov.exchange("cancelled", &a, MULTIPLE, &cancel).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(after, 0);
    cov.same_tables("cancelled").await;
    assert_eq!(cov.advance("past the hold-off", 15).await, 0);
    cov.stop().await;
}

/// A cancelled context's report runs on, on both sides, and its failure
/// holds nothing off for the context made again meanwhile (#2419). The new
/// subscription's own report is refused first, so its follow-up is due
/// when that hold-off ends, 12 s on, and the old report's refusal 5 s
/// later moves it on neither side.
#[tokio::test(start_paused = true)]
async fn a_cancelled_contexts_failed_report_holds_nothing_off_on_either_side() {
    let mut cov = CovParity::start_multiple().await;
    let a = direct(1);
    let timed = multiple(1, true, &[(ai(), true)], Some((600, DELAY)));
    assert_eq!(cov.exchange("confirmed", &a, MULTIPLE, &timed).await.1, 1);
    let cancel = multiple(1, true, &[], None);
    assert_eq!(cov.exchange("cancelled", &a, MULTIPLE, &cancel).await.1, 0);
    assert_eq!(cov.exchange("made again", &a, MULTIPLE, &timed).await.1, 1);
    cov.same_tables("made again").await;

    // The newer of the two reports out is answered first.
    cov.outstanding.rotate_left(1);
    assert_eq!(cov.answer("the new report refused", true).await, 0);
    // The cancelled subscription's report is sent again at 3 s.
    assert_eq!(cov.advance("the old report, resent", 5).await, 1);
    assert_eq!(cov.answer("the old report refused", true).await, 0);
    assert_eq!(cov.advance("the new report's hold-off", 6).await, 0);
    assert_eq!(cov.advance("its end", 2).await, 1);
    assert_eq!(cov.answer("Ack of the follow-up", false).await, 0);
    cov.stop().await;
}

/// A report the backstop sent is refused: its follow-up goes out when the
/// hold-off ends on both sides, not before and not a whole
/// Max_Notification_Delay later (#2419). The delay here is longer than
/// the 12 s hold-off, so a refused report's change waits for the backstop.
#[tokio::test(start_paused = true)]
async fn a_refused_backstop_report_follows_its_hold_off_on_both_sides() {
    const SLOW: u32 = 30;
    let mut cov = CovParity::start_multiple().await;
    let a = direct(1);
    let timed = multiple(1, true, &[(ai(), true)], Some((600, SLOW)));
    assert_eq!(cov.exchange("confirmed", &a, MULTIPLE, &timed).await.1, 1);
    assert_eq!(cov.answer("Ack", false).await, 0);
    let oos = PropertyValue::Boolean(true);
    assert_eq!(cov.write("Status_Flags", &a, ai(), OOS, oos).await, 1);
    assert_eq!(cov.answer("Ack", false).await, 0);
    assert_eq!(cov.sample("to refuse", 40.0, false).await, 1);
    assert_eq!(cov.answer("refused", true).await, 0);
    assert_eq!(cov.advance("past its hold-off", 29).await, 0);
    assert_eq!(cov.advance("the backstop", 2).await, 1);

    assert_eq!(cov.answer("the backstop's report refused", true).await, 0);
    assert_eq!(cov.advance("in its hold-off", 11).await, 0);
    assert_eq!(cov.advance("the hold-off's end", 2).await, 1);
    assert_eq!(cov.answer("Ack of the follow-up", false).await, 0);
    cov.stop().await;
}

/// A refused report's references are listed again inside its hold-off
/// (#2419): on both sides the new subscription reports at once, carrying
/// the refused report's change, and nothing more goes out when the old
/// hold-off would have ended.
#[tokio::test(start_paused = true)]
async fn a_context_listed_again_in_a_hold_off_starts_afresh_on_both_sides() {
    let mut cov = CovParity::start_multiple().await;
    let a = direct(1);
    let timed = multiple(1, true, &[(ai(), true)], Some((600, DELAY)));
    assert_eq!(cov.exchange("confirmed", &a, MULTIPLE, &timed).await.1, 1);
    assert_eq!(cov.answer("refused", true).await, 0);
    assert_eq!(cov.advance("into the hold-off", 1).await, 0);
    assert_eq!(
        cov.exchange("listed again", &a, MULTIPLE, &timed).await.1,
        1
    );
    cov.same_tables("listed again").await;
    assert_eq!(cov.answer("Ack", false).await, 0);
    assert_eq!(cov.advance("past the old hold-off's end", 15).await, 0);
    cov.stop().await;
}

/// A follow-up report lists its untimestamped items in key order on both
/// sides (#2320): two references of one confirmed context change while its
/// initial report is out, and the Ack's follow-up carries both. The
/// server's follow-up queue is a hash set, so before its keys were sorted
/// the two items came out either way round.
#[tokio::test(start_paused = true)]
async fn a_follow_up_lists_its_items_in_key_order() {
    for _ in 0..8 {
        let mut cov = CovParity::start_multiple().await;
        let a = direct(1);
        let both = multiple(
            1,
            true,
            &[(msv(), false), (ai(), false)],
            Some((600, DELAY)),
        );
        assert_eq!(cov.exchange("confirmed", &a, MULTIPLE, &both).await.1, 1);
        let oos = PropertyValue::Boolean(true);
        assert_eq!(cov.write("AI-1 held", &a, ai(), OOS, oos).await, 0);
        let state = PropertyValue::Unsigned(2);
        assert_eq!(cov.write("MSV-1 held", &a, msv(), PV, state).await, 0);
        assert_eq!(cov.answer("Ack, then both items", false).await, 1);
        assert_eq!(cov.answer("Ack", false).await, 0);
        cov.stop().await;
    }
}

/// Under a spent budget both sides keep back the same contexts and send
/// the rest in the same order (#2320): two notifications a change for
/// three contexts, whose turns start from a place that moves per fanout,
/// by one sequence in the core that both hosts advance.
#[tokio::test(start_paused = true)]
async fn a_spent_budget_keeps_back_the_same_contexts_on_both_sides() {
    let policy = CovPolicy {
        max_notifications_per_event: 2,
        ..CovPolicy::default()
    };
    let mut cov = CovParity::start_multiple_under(policy).await;
    let a = direct(1);
    for process in [2, 3, 1] {
        let body = multiple(process, false, &[(ai(), false)], Some((300, DELAY)));
        assert_eq!(cov.exchange("subscribed", &a, MULTIPLE, &body).await.1, 1);
    }
    let oos = PropertyValue::Boolean(true);
    assert_eq!(cov.write("Status_Flags", &a, ai(), OOS, oos).await, 2);
    for change in 0..8u8 {
        let value = 30.0 + f32::from(change);
        assert_eq!(cov.sample("two of three", value, false).await, 2);
    }
    cov.same_tables("at the end").await;
    cov.stop().await;
}
