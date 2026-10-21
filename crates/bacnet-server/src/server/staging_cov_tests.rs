//! Staging COV reporting over the wire (#988).
//!
//! Table 13-1 has a Staging notification fire when Present_Value moves by
//! COV_Increment, when Status_Flags changes, or when Present_Stage changes,
//! and carry Present_Value, Status_Flags and Present_Stage. STG-5 below drives
//! BO-5, which is missing at startup, so the startup plan faults the source.
//! A Present_Value write that changes the stage queues writes to the stage's
//! targets; completing them sets the source's Reliability, and a change there
//! (the source completion) reaches subscribers as a Status_Flags change.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_services::cov::{
    COVNotificationRequest, SubscribeCOVPropertyRequest, SubscribeCOVRequest,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use std::collections::BTreeMap;

const STAGE: PropertyIdentifier = PropertyIdentifier::PRESENT_STAGE;
const FAULT: u8 = 0x4;

type Values = Vec<(PropertyIdentifier, Vec<u8>)>;

fn stg5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 5).unwrap()
}

fn bo5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 5).unwrap()
}

fn encode(value: PropertyValue) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &value).unwrap();
    encoded.to_vec()
}

/// Encoded Status_Flags with `bits` in the high nibble (FAULT = 0x4).
fn flags(bits: u8) -> Vec<u8> {
    encode(PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits << 4],
    })
}

fn stage(value: u64) -> Vec<u8> {
    encode(PropertyValue::Unsigned(value))
}

/// The full Table 13-1 Staging report.
fn staging_report(pv: f32, status: u8, present_stage: u64) -> Values {
    vec![
        (PV, real(pv)),
        (SF, flags(status)),
        (STAGE, stage(present_stage)),
    ]
}

/// STG-5: stages up to 10, 30 and 50 with deadband 1, BO-5 INACTIVE in the
/// first stage and ACTIVE in the others; Present_Value starts at 5 (stage 1).
fn staging(db: &mut ObjectDatabase) {
    let stage = |limit: f32, active: bool| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    db.add(Box::new(
        StagingObject::new(
            5,
            "STG-5",
            StagingConfig {
                present_value: 5.0,
                min_present_value: 0.0,
                units: EngineeringUnits::DEGREES_CELSIUS,
                priority_for_writing: 8,
                stages: vec![stage(10.0, false), stage(30.0, true), stage(50.0, true)],
                target_references: vec![BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: bo5(),
                }],
                stage_names: None,
            },
        )
        .unwrap(),
    ))
    .unwrap();
}

/// A server whose STG-5 reports PV moves of at least 10.
async fn start() -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), staging).await;
    write_staging(
        &mut h,
        PropertyIdentifier::COV_INCREMENT,
        PropertyValue::Real(10.0),
    )
    .await;
    assert_eq!(response(&h).await, Ok(()));
    h
}

async fn write_staging(h: &mut Harness, property: PropertyIdentifier, value: PropertyValue) {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: stg5(),
        property_identifier: property,
        property_array_index: None,
        property_value: encode(value),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
}

async fn write_pv(h: &mut Harness, value: f32) {
    write_staging(h, PV, PropertyValue::Real(value)).await;
    assert_eq!(response(h).await, Ok(()));
}

fn values(notification: &COVNotificationRequest) -> Values {
    assert_eq!(notification.monitored_object_identifier, stg5());
    notification
        .list_of_values
        .iter()
        .map(|value| {
            assert_eq!(value.property_array_index, None);
            (value.property_identifier, value.value.clone())
        })
        .collect()
}

/// The next `count` notifications keyed by subscriber process, then none.
async fn reports(h: &Harness, count: usize) -> BTreeMap<u32, Values> {
    let mut reports = BTreeMap::new();
    for _ in 0..count {
        let notification = h.cov_notification().await;
        let process = notification.subscriber_process_identifier;
        assert!(
            reports.insert(process, values(&notification)).is_none(),
            "process {process} reported twice"
        );
    }
    h.no_notification().await;
    reports
}

async fn target(h: &Harness) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&bo5())
        .unwrap()
        .read_property(PV, None)
        .unwrap()
}

async fn add_target(h: &Harness) {
    h.server
        .database()
        .write()
        .await
        .add(Box::new(BinaryOutputObject::new(5, "BO-5").unwrap()))
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn staging_subscribe_cov_reports_stage_increment_and_completion() {
    let mut h = start().await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 988,
        monitored_object_identifier: stg5(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    assert_eq!(response(&h).await, Ok(()));
    let initial = h.cov_notification().await;
    assert_eq!(initial.subscriber_process_identifier, 988);
    assert_eq!(
        values(&initial),
        staging_report(5.0, FAULT, 1),
        "the startup plan could not reach BO-5"
    );

    // Entering stage 2 writes BO-5 ACTIVE, and that completion clears FAULT.
    // A WriteProperty runs the plan before its own fanout, so one report
    // carries both changes.
    add_target(&h).await;
    write_pv(&mut h, 11.5).await;
    assert_eq!(
        values(&h.cov_notification().await),
        staging_report(11.5, 0, 2)
    );
    assert_eq!(target(&h).await, PropertyValue::Enumerated(1));
    h.no_notification().await;

    // Within stage 2 only a move of at least COV_Increment reports.
    write_pv(&mut h, 15.0).await;
    h.no_notification().await;
    write_pv(&mut h, 25.0).await;
    assert_eq!(
        values(&h.cov_notification().await),
        staging_report(25.0, 0, 2)
    );
    h.no_notification().await;

    // Entering stage 3 moves Present_Value by only 6.5 and the plan succeeds
    // without a Reliability change: the Present_Stage change alone reports.
    write_pv(&mut h, 31.5).await;
    assert_eq!(
        values(&h.cov_notification().await),
        staging_report(31.5, 0, 3)
    );
    h.no_notification().await;

    // Back in stage 2 the plan cannot reach the removed BO-5, so its
    // completion faults the source.
    h.server.database().write().await.remove(&bo5()).unwrap();
    write_pv(&mut h, 28.5).await;
    assert_eq!(
        values(&h.cov_notification().await),
        staging_report(28.5, FAULT, 2)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn staging_subscribe_cov_property_reports_each_property_with_flags() {
    let mut h = start().await;
    // Process 1 watches Present_Value (inheriting COV_Increment), 2
    // Status_Flags and 3 Present_Stage.
    for (process, property) in [(1, PV), (2, SF), (3, STAGE)] {
        let mut body = BytesMut::new();
        SubscribeCOVPropertyRequest {
            subscriber_process_identifier: process,
            monitored_object_identifier: stg5(),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
            monitored_property_identifier: property,
            monitored_property_array_index: None,
            cov_increment: None,
        }
        .encode(&mut body)
        .unwrap();
        h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
            .await;
        assert_eq!(response(&h).await, Ok(()), "{property:?}");
    }
    let pv = |value: f32, status: u8| vec![(PV, real(value)), (SF, flags(status))];
    let sf = |status: u8| vec![(SF, flags(status))];
    let st = |value: u64, status: u8| vec![(STAGE, stage(value)), (SF, flags(status))];
    assert_eq!(
        reports(&h, 3).await,
        BTreeMap::from([(1, pv(5.0, FAULT)), (2, sf(FAULT)), (3, st(1, FAULT))])
    );

    // The completion that clears FAULT reaches every property.
    add_target(&h).await;
    write_pv(&mut h, 11.5).await;
    assert_eq!(
        reports(&h, 3).await,
        BTreeMap::from([(1, pv(11.5, 0)), (2, sf(0)), (3, st(2, 0))])
    );

    // A Present_Value move reaches only its watcher, past the increment.
    write_pv(&mut h, 15.0).await;
    h.no_notification().await;
    write_pv(&mut h, 25.0).await;
    assert_eq!(reports(&h, 1).await, BTreeMap::from([(1, pv(25.0, 0))]));

    // A stage change alone reaches only the Present_Stage watcher.
    write_pv(&mut h, 31.5).await;
    assert_eq!(reports(&h, 1).await, BTreeMap::from([(3, st(3, 0))]));
    assert_eq!(target(&h).await, PropertyValue::Enumerated(1));

    // The faulting completion reaches every property again.
    h.server.database().write().await.remove(&bo5()).unwrap();
    write_pv(&mut h, 28.5).await;
    assert_eq!(
        reports(&h, 3).await,
        BTreeMap::from([(1, pv(28.5, FAULT)), (2, sf(FAULT)), (3, st(2, FAULT))])
    );
    h.server.stop().await.unwrap();
}
