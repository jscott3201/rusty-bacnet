//! Unit tests of `evaluate`: what a change reports, against the last
//! delivered observation, within the input's budget and the flights.

use super::*;
use crate::cov::life_safety::LifeSafetyCovSnapshots;
use crate::cov::{CovNotificationKind, CovPolicy, CovSubscription, Delivery};
use PropertyIdentifier as P;
use alloc::borrow::Cow;
use bacnet_encoding::apdu::{
    Apdu, ConfirmedRequest as ConfirmedRequestPdu, UnconfirmedRequest, encode_apdu,
};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::CovReporting;
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ErrorClass, ErrorCode, LifeSafetyState, ObjectType,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

const NOW: Instant = Instant::from_secs(1_000);

/// An Analog Input whose COV values the test sets: Present_Value, the
/// Status_Flags bits, a Table 13-1 trigger (Present_Stage) and a value that
/// rides along (Setpoint), named by its own reported list.
#[derive(Clone)]
struct Probe {
    pv: f32,
    flags: u8,
    stage: u32,
    setpoint: f32,
    increment: Option<f64>,
}

impl Probe {
    fn new() -> Self {
        Self {
            pv: 10.0,
            flags: 0,
            stage: 1,
            setpoint: 20.0,
            increment: Some(1.0),
        }
    }
}

impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
    }
    fn object_name(&self) -> &str {
        "probe"
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::PRESENT_STAGE,
            P::SETPOINT,
        ])
    }
    fn read_property(&self, p: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        Ok(match p {
            P::PRESENT_VALUE => PropertyValue::Real(self.pv),
            P::STATUS_FLAGS => PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.flags << 4],
            },
            P::PRESENT_STAGE => PropertyValue::Unsigned(u64::from(self.stage)),
            P::SETPOINT => PropertyValue::Real(self.setpoint),
            _ => {
                return Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                ));
            }
        })
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        Some(self)
    }
}

impl CovReporting for Probe {
    fn cov_increment(&self) -> Option<f64> {
        self.increment
    }
    fn cov_reported_properties(&self) -> Option<&'static [CovReportedProperty]> {
        Some(&[
            CovReportedProperty::Trigger(P::PRESENT_STAGE),
            CovReportedProperty::Value(P::SETPOINT),
        ])
    }
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()
}

/// A subscription to `object` from process `process`: whole-object when
/// `property` is `None`.
fn proposal(
    object: ObjectIdentifier,
    process: u32,
    property: Option<P>,
    confirmed: bool,
) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[0x0A, 0, 0, 5, 0xBA, 0xC0]),
        subscriber_network: None,
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: confirmed,
        expires_at: Some(NOW + bacnet_time::Duration::from_secs(300)),
        last_notified_observation: None,
        monitored_property: property,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    }
}

/// The table's live subscriptions to `object`, as a host reads them for
/// each change.
fn live(table: &mut CovTable, object: ObjectIdentifier) -> Vec<CovSubscriptionSnapshot> {
    table
        .subscriptions_for(&object, NOW)
        .into_iter()
        .cloned()
        .collect()
}

/// Evaluate a change of `object` for all its subscriptions under a full
/// budget, admitting every confirmed report.
fn changed(table: &mut CovTable, object: &dyn BACnetObject, force: bool) -> Vec<CovReport<()>> {
    let subs = live(table, object.object_identifier());
    let mut budget = EventBudget::new(table.policy());
    evaluate(
        table,
        object,
        &subs,
        Evaluation {
            device: device(),
            force,
        },
        &mut budget,
        NOW,
        |_, _| Some(()),
    )
}

/// Each report's subscriber process and its values' properties.
fn summary(reports: &[CovReport<()>]) -> Vec<(u32, Vec<P>)> {
    reports
        .iter()
        .map(|report| {
            let request = COVNotificationRequest::decode(&report.params).unwrap();
            let properties = request
                .list_of_values
                .iter()
                .map(|value| value.property_identifier)
                .collect();
            (request.subscriber_process_identifier, properties)
        })
        .collect()
}

/// The encoded Present_Value a report carries.
fn reported_pv(report: &CovReport<()>) -> Vec<u8> {
    COVNotificationRequest::decode(&report.params)
        .unwrap()
        .list_of_values
        .into_iter()
        .find(|value| value.property_identifier == P::PRESENT_VALUE)
        .unwrap()
        .value
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

/// Send every report: its observation becomes the baseline.
fn sent(table: &mut CovTable, reports: Vec<CovReport<()>>) {
    for report in reports {
        assert!(
            table
                .delivered(report.token, Delivery::Sent, NOW)
                .is_empty()
        );
    }
}

#[test]
fn an_increment_reports_at_and_over_it_and_not_under() {
    let mut table = CovTable::default();
    let mut probe = Probe::new();
    table
        .subscribe(proposal(probe.object_identifier(), 1, None, false), NOW)
        .unwrap();
    let initial = changed(&mut table, &probe, true);
    assert_eq!(initial.len(), 1, "the initial notification");
    sent(&mut table, initial);

    probe.pv = 10.5;
    assert!(changed(&mut table, &probe, false).is_empty(), "under");
    // Exactly the increment away reports (Table 13-1: by COV_Increment
    // or more).
    probe.pv = 11.0;
    let at = changed(&mut table, &probe, false);
    assert_eq!(at.len(), 1, "at");
    assert_eq!(reported_pv(&at[0]), encoded(&PropertyValue::Real(11.0)));
    // Not sent: the baseline stays 10.0.
    assert!(
        table
            .delivered(at[0].token, Delivery::NotSent, NOW)
            .is_empty()
    );
    probe.pv = 9.5;
    assert!(changed(&mut table, &probe, false).is_empty(), "under, down");
    probe.pv = 11.5;
    assert_eq!(changed(&mut table, &probe, false).len(), 1, "over");
}

#[test]
fn status_flags_alone_report_to_both_forms() {
    let mut table = CovTable::default();
    let mut probe = Probe::new();
    let oid = probe.object_identifier();
    table.subscribe(proposal(oid, 1, None, false), NOW).unwrap();
    table
        .subscribe(proposal(oid, 2, Some(P::PRESENT_VALUE), false), NOW)
        .unwrap();
    let initial = changed(&mut table, &probe, true);
    assert_eq!(initial.len(), 2);
    sent(&mut table, initial);

    probe.flags = 0b0100; // FAULT
    let reports = changed(&mut table, &probe, false);
    assert_eq!(
        summary(&reports),
        [
            (
                1,
                vec![
                    P::PRESENT_VALUE,
                    P::STATUS_FLAGS,
                    P::PRESENT_STAGE,
                    P::SETPOINT
                ]
            ),
            (2, vec![P::PRESENT_VALUE, P::STATUS_FLAGS]),
        ]
    );
}

#[test]
fn a_table_13_1_trigger_reports_and_a_ride_along_value_does_not() {
    let mut table = CovTable::default();
    let mut probe = Probe::new();
    table
        .subscribe(proposal(probe.object_identifier(), 1, None, false), NOW)
        .unwrap();
    let initial = changed(&mut table, &probe, true);
    sent(&mut table, initial);

    probe.setpoint = 25.0;
    assert!(
        changed(&mut table, &probe, false).is_empty(),
        "Setpoint rides along"
    );
    probe.stage = 2;
    let reports = changed(&mut table, &probe, false);
    assert_eq!(reports.len(), 1, "Present_Stage triggers");
    let values = COVNotificationRequest::decode(&reports[0].params)
        .unwrap()
        .list_of_values;
    assert_eq!(values[3].property_identifier, P::SETPOINT);
    assert_eq!(
        values[3].value,
        encoded(&PropertyValue::Real(25.0)),
        "the ride-along value goes out with it"
    );
}

#[test]
fn a_property_subscription_measures_with_its_own_increment() {
    let mut table = CovTable::default();
    let mut probe = Probe::new();
    let oid = probe.object_identifier();
    let mut own = proposal(oid, 1, Some(P::PRESENT_VALUE), false);
    own.cov_increment = Some(0.25);
    table.subscribe(own, NOW).unwrap();
    // The object's increment applies to one that names none.
    table
        .subscribe(proposal(oid, 2, Some(P::PRESENT_VALUE), false), NOW)
        .unwrap();
    let initial = changed(&mut table, &probe, true);
    sent(&mut table, initial);

    probe.pv = 10.5;
    assert_eq!(
        summary(&changed(&mut table, &probe, false)),
        [(1, vec![P::PRESENT_VALUE, P::STATUS_FLAGS])]
    );
}

#[test]
fn value_source_takes_the_table_13_1a_2_report() {
    let mut table = CovTable::default();
    let mut value = AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap();
    let oid = value.object_identifier();
    table
        .subscribe(proposal(oid, 1, Some(P::VALUE_SOURCE), false), NOW)
        .unwrap();
    let initial = changed(&mut table, &value, true);
    assert_eq!(
        summary(&initial),
        [(
            1,
            vec![
                P::PRESENT_VALUE,
                P::STATUS_FLAGS,
                P::VALUE_SOURCE,
                P::LAST_COMMAND_TIME,
                P::CURRENT_COMMAND_PRIORITY,
            ]
        )]
    );
    sent(&mut table, initial);
    assert!(changed(&mut table, &value, false).is_empty(), "unchanged");

    // A command at priority 8 moves the commanding priority.
    let origin = bacnet_objects::command_source::CommandOrigin::Local {
        owner_device: device(),
        initiating_object: None,
    };
    value
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Real(0.0),
            Some(8),
            &origin,
        )
        .unwrap();
    assert_eq!(changed(&mut table, &value, false).len(), 1);
}

#[test]
fn a_life_safety_change_reports_from_its_snapshot() {
    // The point before and after the mutation, each in its own database.
    let database = |state| {
        let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        point.set_present_value(state);
        let mut db = bacnet_objects::database::ObjectDatabase::new();
        db.add(Box::new(point)).unwrap();
        db
    };
    let db = database(LifeSafetyState::QUIET);
    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut table = CovTable::default();
    table.subscribe(proposal(oid, 1, None, false), NOW).unwrap();
    table
        .subscribe(proposal(oid, 2, Some(P::SILENCED), false), NOW)
        .unwrap();
    let initial = changed(&mut table, db.get(&oid).unwrap(), true);
    sent(&mut table, initial);

    let before = LifeSafetyCovSnapshots::capture_oid(&db, oid);
    let db = database(LifeSafetyState::ALARM);
    let changes = before.changes(&db, &[oid]);
    let [change] = changes.as_slice() else {
        panic!("one changed point: {changes:?}")
    };
    assert!(change.changed_properties.contains(&P::PRESENT_VALUE));
    assert!(!change.status_changed());
    let subs: Vec<_> = live(&mut table, oid)
        .into_iter()
        .filter(|sub| change.notifies(sub))
        .collect();
    assert_eq!(subs.len(), 1, "Silenced didn't change");
    let mut budget = EventBudget::new(table.policy());
    let reports = evaluate(
        &mut table,
        db.get(&oid).unwrap(),
        &subs,
        Evaluation {
            device: device(),
            force: change.status_changed(),
        },
        &mut budget,
        NOW,
        |_, _| Some(()),
    );
    assert_eq!(summary(&reports)[0].0, 1, "the whole-object subscription");
    assert_eq!(
        reported_pv(&reports[0]),
        encoded(&PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw()))
    );
}

#[test]
fn the_budget_cuts_off_what_one_change_sends_and_counts_it() {
    let mut table = CovTable::new(CovPolicy {
        max_notifications_per_event: 2,
        ..CovPolicy::default()
    });
    let probe = Probe::new();
    for process in 1..=3 {
        table
            .subscribe(
                proposal(probe.object_identifier(), process, None, false),
                NOW,
            )
            .unwrap();
    }
    let reports = changed(&mut table, &probe, true);
    assert_eq!(reports.len(), 2);
    assert_eq!(table.take_counts().notifications_throttled_fanout, 1);

    // Octets too: a budget one octet short of the second APDU.
    let subs = live(&mut table, probe.object_identifier());
    let octets = reports[0].params.len() + UNCONFIRMED_HEADER;
    let mut budget = EventBudget::with_limits(10, octets * 2 - 1);
    let evaluation = Evaluation {
        device: device(),
        force: true,
    };
    let cut = evaluate(
        &mut table,
        &probe,
        &subs,
        evaluation,
        &mut budget,
        NOW,
        |_, _| Some(()),
    );
    assert_eq!(cut.len(), 1);
    assert_eq!(budget.remaining_bytes(), octets - 1);
    assert_eq!(table.take_counts().notifications_throttled_fanout, 2);
}

#[test]
fn a_change_while_a_report_is_out_is_owed_to_its_ack() {
    let mut table = CovTable::default();
    let mut probe = Probe::new();
    let oid = probe.object_identifier();
    let key = table
        .subscribe(proposal(oid, 1, None, true), NOW)
        .unwrap()
        .key()
        .clone();
    let initial = changed(&mut table, &probe, true);
    assert_eq!(
        table.delivered(initial[0].token, Delivery::Acknowledged, NOW),
        core::slice::from_ref(&key)
    );

    probe.pv = 12.0;
    let out = changed(&mut table, &probe, false);
    assert_eq!(out.len(), 1);
    probe.pv = 14.0;
    // Held while the report is out, before the host is asked for an
    // in-flight slot or an invoke ID.
    let subs = live(&mut table, oid);
    let mut budget = EventBudget::new(table.policy());
    let mut admitted = 0;
    let held = evaluate(
        &mut table,
        &probe,
        &subs,
        Evaluation {
            device: device(),
            force: false,
        },
        &mut budget,
        NOW,
        |_, _| {
            admitted += 1;
            Some(())
        },
    );
    assert!(held.is_empty());
    assert_eq!(admitted, 0, "no admission for a busy subscription");
    assert_eq!(table.take_counts().notifications_throttled_fanout, 0);
    // The Ack commits 12.0 and owes the subscription a fresh look, which
    // reports 14.0.
    assert_eq!(
        table.delivered(out[0].token, Delivery::Acknowledged, NOW),
        [key]
    );
    let owed = changed(&mut table, &probe, false);
    assert_eq!(reported_pv(&owed[0]), encoded(&PropertyValue::Real(14.0)));
}

#[test]
fn a_refused_admission_drops_the_report_unspent() {
    let mut table = CovTable::default();
    let probe = Probe::new();
    let oid = probe.object_identifier();
    let confirmed = table.subscribe(proposal(oid, 1, None, true), NOW).unwrap();
    table.subscribe(proposal(oid, 2, None, false), NOW).unwrap();
    let subs = live(&mut table, oid);
    let mut budget = EventBudget::new(table.policy());
    let evaluation = Evaluation {
        device: device(),
        force: true,
    };
    // Every report is offered with its APDU's octets, confirmed or not.
    let mut offered = Vec::new();
    let reports = evaluate(
        &mut table,
        &probe,
        &subs,
        evaluation,
        &mut budget,
        NOW,
        |sub, octets| {
            offered.push((sub.subscriber_process_identifier, octets));
            None::<()>
        },
    );
    assert!(reports.is_empty());
    let header = |process| {
        if process == 1 {
            CONFIRMED_HEADER
        } else {
            UNCONFIRMED_HEADER
        }
    };
    offered.sort_unstable();
    assert_eq!(offered.len(), 2);
    assert_eq!(offered[0].1 - header(1), offered[1].1 - header(2));
    assert_eq!(budget.remaining_notifications(), 64);
    assert!(table.confirmed_idle(&confirmed, NOW), "no flight began");
    assert!(table.pending.is_empty(), "nothing awaits a send");
}

#[test]
fn an_unsent_report_leaves_the_baseline() {
    let mut table = CovTable::default();
    let mut probe = Probe::new();
    table
        .subscribe(proposal(probe.object_identifier(), 1, None, false), NOW)
        .unwrap();
    let initial = changed(&mut table, &probe, true);
    sent(&mut table, initial);
    probe.pv = 12.0;
    let report = changed(&mut table, &probe, false).remove(0);
    table.delivered(report.token, Delivery::NotSent, NOW);
    assert!(table.pending.is_empty());
    // Still measured from 10.0, and a second end of the same report
    // changes nothing.
    probe.pv = 10.5;
    assert!(changed(&mut table, &probe, false).is_empty());
    table.delivered(report.token, Delivery::Sent, NOW);
    probe.pv = 11.0;
    assert_eq!(changed(&mut table, &probe, false).len(), 1);
}

#[test]
fn the_header_octets_match_the_encoded_apdus() {
    let params = Bytes::from_static(&[0x09, 0x01]);
    let mut buf = BytesMut::new();
    encode_apdu(
        &mut buf,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            service_request: params.clone(),
        }),
    )
    .unwrap();
    assert_eq!(buf.len(), UNCONFIRMED_HEADER + params.len());
    buf.clear();
    encode_apdu(
        &mut buf,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: params.clone(),
        }),
    )
    .unwrap();
    assert_eq!(buf.len(), CONFIRMED_HEADER + params.len());
}
