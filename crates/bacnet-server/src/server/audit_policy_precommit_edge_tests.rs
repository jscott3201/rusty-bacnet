//! Timestamp and late-preparation failures at the same immediate owner.
use super::*;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_types::primitives::{Date, Time};

struct FixedClock;
impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

#[tokio::test]
async fn mandatory_policy_datetime_local_identity_and_capacity_rollback() {
    let mut f = server(reporter()).await;
    install(&f, policy()).await;
    f.server
        .db
        .write()
        .await
        .set_clock_reader(Some(Arc::new(FixedClock)));
    let held: Vec<_> = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    let target = oid(ObjectType::ANALOG_VALUE, 11);
    denied(
        f.server
            .write_local(
                &target,
                PropertyIdentifier::AUDITABLE_OPERATIONS,
                None,
                change_value(PropertyIdentifier::AUDITABLE_OPERATIONS),
                None,
            )
            .await,
    );
    assert_unchanged(&f, ObjectType::ANALOG_VALUE, policy(), 0).await;
    drop(held);
    f.server
        .write_local(
            &target,
            PropertyIdentifier::AUDITABLE_OPERATIONS,
            None,
            change_value(PropertyIdentifier::AUDITABLE_OPERATIONS),
            None,
        )
        .await
        .unwrap();
    settle().await;
    let records = notifications(&f.transport.sent);
    assert_eq!(records.len(), 1);
    let record = &records[0].notifications[0];
    let frame = FixedClock.read_clock().unwrap();
    assert_eq!(
        record.target_timestamp,
        Some(BACnetTimeStamp::DateTime {
            date: frame.local_date,
            time: frame.local_time
        })
    );
    assert_eq!(
        record.source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(record.invoke_id, None);
    assert_eq!(record.target_object, Some(target));
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn mandatory_policy_late_encoding_denial_releases_confirmed_lease_and_permit() {
    let mut r = reporter();
    r.set_issue_confirmed_notifications(true).unwrap();
    let mut f = server(r).await;
    install(&f, policy()).await;
    // 50 is a supported maximum-APDU setting. This real record does not fit,
    // after its permit and confirmed operation have already been reserved.
    f.server.config.max_apdu_length = 50;
    let response = write(
        &f,
        ObjectType::ANALOG_VALUE,
        PropertyIdentifier::AUDIT_LEVEL,
        PropertyValue::Enumerated(u32::MAX),
        None,
    )
    .await;
    assert!(
        matches!(response, Apdu::Error(ref e) if e.error_class == ErrorClass::SERVICES && e.error_code == ErrorCode::SERVICE_REQUEST_DENIED),
        "{response:?}"
    );
    assert_unchanged(&f, ObjectType::ANALOG_VALUE, policy(), 0).await;
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}
