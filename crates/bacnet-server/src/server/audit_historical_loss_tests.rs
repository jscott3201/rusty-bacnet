//! Boundary tests for the same target/source coalescer, with captured target pins.
use super::*;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_types::{
    bitstring::AuditOperationFlags,
    enums::{AuditLevel, AuditOperation, ObjectType},
    primitives::{Date, Time},
};

fn reporter() -> AuditReporterObject {
    let mut reporter = AuditReporterObject::new(1, "loss").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(flags).unwrap();
    reporter
}
fn context(status: &Arc<AuditReporterStatus>, epoch: u64, route: u8) -> AuditFailureContext<u8> {
    AuditFailureContext {
        status: Arc::clone(status),
        epoch,
        device: ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap(),
        confirmed: false,
        peer: CanonicalPeer::direct(&[route]),
        route,
        max_apdu: 1476,
    }
}
#[tokio::test]
async fn target_historical_loss_survives_disable_aba_and_restores_by_original_order() {
    let owner = NotificationTransactions::new();
    let mut reporter = reporter();
    let status = reporter.status_internal();
    let queue = AuditFailureQueue::target(
        Arc::new(tokio::sync::Semaphore::new(256)),
        status.configuration_epoch(),
        true,
    );
    let first = queue
        .observe(context(&status, status.configuration_epoch(), 1))
        .unwrap();
    let second = queue
        .observe(context(&status, status.configuration_epoch(), 1))
        .unwrap();
    let later = BACnetTimeStamp::DateTime {
        date: Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        },
        time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
    };
    let mut worker = queue.record_drop(&owner, second, later, 2).unwrap();
    let (batch, permit, reserved) = worker.next().await.unwrap();
    let reservation = queue
        .prepare_context(status.next_configuration_epoch().unwrap(), false)
        .unwrap();
    reporter.set_audit_level(AuditLevel::NONE).unwrap();
    reservation.publish();
    assert!(queue
        .record_drop(
            &owner,
            first.clone(),
            BACnetTimeStamp::SequenceNumber(65535),
            1
        )
        .is_none());
    worker.restore(batch);
    drop((permit, reserved));
    let (batch, permit, reserved) = worker.next().await.unwrap();
    assert_eq!(batch.count, 3);
    assert_eq!(batch.earliest, BACnetTimeStamp::SequenceNumber(65535));
    assert_eq!(batch.context.route, 1);
    assert!(
        batch
            .context
            .status
            .begin_auditing_failure_delivery(batch.context.epoch)
            .is_none(),
        "historical delivery has no current health authority"
    );
    drop((batch, permit, reserved));
    let reservation = queue
        .prepare_context(status.next_configuration_epoch().unwrap(), true)
        .unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    reservation.publish();
    let new = queue
        .observe(context(&status, status.configuration_epoch(), 1))
        .unwrap();
    assert!(!Arc::ptr_eq(&first.context, &new.context));
    assert!(queue
        .record_drop(&owner, new, BACnetTimeStamp::SequenceNumber(0), 1)
        .is_none());
    assert!(queue
        .record_drop(&owner, first, BACnetTimeStamp::SequenceNumber(65535), 1)
        .is_none());
    let (old, _, _) = worker.next().await.unwrap();
    assert_eq!(old.earliest, BACnetTimeStamp::SequenceNumber(65535));
    let (new, _, _) = worker.next().await.unwrap();
    assert_eq!(new.earliest, BACnetTimeStamp::SequenceNumber(0));
    assert!(worker.next().await.is_none());
    owner.close();
}
#[test]
fn target_historical_context_global_and_reporter_limits_have_rollback_owned_reservations() {
    let budget = Arc::new(tokio::sync::Semaphore::new(256));
    let status = reporter().status_internal();
    let queues = (0..64)
        .map(|_| AuditFailureQueue::target(Arc::clone(&budget), 0, true))
        .collect::<Vec<_>>();
    let mut pins = vec![];
    for queue in &queues {
        for epoch in 0..4 {
            if epoch != 0 {
                queue.prepare_context(epoch, true).unwrap().publish();
            }
            pins.push(queue.observe(context(&status, epoch, 1)).unwrap());
        }
    }
    assert_eq!(budget.available_permits(), 0);
    assert!(queues[0].prepare_context(4, true).is_err());
    assert_eq!(queues[0].state.lock().unwrap().reserved, 0);
    pins.remove(0); // Reclaim the released historical context only, never the current baseline.
    let prepared = queues[0].prepare_context(4, true).unwrap();
    assert_eq!(budget.available_permits(), 0);
    drop(prepared);
    assert_eq!(budget.available_permits(), 1);
    assert_eq!(queues[0].state.lock().unwrap().current, Some(3));
    drop((pins, queues));
    assert_eq!(budget.available_permits(), 256);
    let queue = AuditFailureQueue::target(Arc::clone(&budget), 0, true);
    let mut pins = vec![];
    for epoch in 0..8 {
        if epoch != 0 {
            queue.prepare_context(epoch, true).unwrap().publish();
        }
        pins.push(queue.observe(context(&status, epoch, 1)).unwrap());
    }
    assert!(queue.prepare_context(8, true).is_err());
    assert_eq!(budget.available_permits(), 248);
    queue.state.lock().unwrap().order = u64::MAX;
    assert!(queue.observe(context(&status, 7, 1)).is_none());
    assert_eq!(queue.state.lock().unwrap().order, u64::MAX);
}
