//! Storage-accounting boundary tests; wire encoding is exercised by the batch suites.
use super::*;
use bacnet_objects::audit::{AuditReporterObject, TargetAuditAssociation};

fn queue() -> (Arc<AuditBatchQueue>, Vec<Arc<AuditReporterStatus>>) {
    let reporters = (1..=5)
        .map(|instance| AuditReporterObject::new(instance, "budget").unwrap())
        .collect::<Vec<_>>();
    let statuses = reporters
        .iter()
        .map(|r| r.status_internal())
        .collect::<Vec<_>>();
    let association = TargetAuditAssociation::new(
        reporters
            .iter()
            .map(|r| {
                use bacnet_objects::traits::BACnetObject;
                (r.object_identifier(), r.status_internal())
            })
            .collect(),
    );
    (AuditBatchQueue::new(&association), statuses)
}
fn record(status: &Arc<AuditReporterStatus>, charge: usize) -> QueuedAudit {
    // The queue receives pre-encoded validated bytes; these synthetic payloads
    // isolate exact accounting without adding an artificial producer datatype.
    let context = super::super::notification_transactions::AuditFailureContext {
        status: Arc::clone(status),
        epoch: status.configuration_epoch(),
        device: ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 10).unwrap(),
        confirmed: false,
        peer: canonical_direct_peer(&[1]),
        route: Arc::new(ConfirmedRecipientRoute {
            canonical_peer: canonical_direct_peer(&[1]),
            local_target: Some(bacnet_types::MacAddr::from_slice(&[1])),
            remote: None,
            freshness: None,
        }),
        max_apdu: 1476,
    };
    let owner = NotificationTransactions::new();
    owner.install_target_audit(TargetAuditAssociation::new(vec![(
        ObjectIdentifier::new(bacnet_types::enums::ObjectType::AUDIT_REPORTER, 1).unwrap(),
        Arc::clone(status),
    )]));
    let ticket = owner
        .audit_failure_queue(status)
        .unwrap()
        .observe(context)
        .unwrap();
    QueuedAudit::new(
        DeliveryCompletion {
            status: Arc::clone(status),
            epoch: status.begin_delivery(),
            finished: false,
        },
        BytesMut::from(vec![0; charge - 4].as_slice()),
        BACnetTimeStamp::SequenceNumber(0),
        ticket,
        AuditSendDelay::new(3600).unwrap(),
    )
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_queue_exact_global_and_per_reporter_records_bytes_and_release() {
    let (queue, statuses) = queue();
    for status in &statuses[..4] {
        for _ in 0..64 {
            assert!(queue.enqueue(record(status, 1024)).is_ok());
        }
        assert!(queue.enqueue(record(status, 4)).is_err());
    }
    assert_eq!(queue.resources(), (256, 256 * 1024, 0));
    assert!(queue.enqueue(record(&statuses[4], 4)).is_err());
    queue.begin_stop();
    let records = queue.take_due().unwrap();
    let retired = records.len();
    let local = LocalDisposition::new(Arc::clone(&queue), &records);
    assert_eq!(
        queue.resources().0,
        256,
        "charge remains through local send"
    );
    drop(local);
    assert_eq!(queue.resources().0, 256 - retired);
    drop(records);
    let pending = queue.close();
    assert_eq!(pending.len(), 256 - retired);
    assert_eq!(queue.resources(), (0, 0, (256 - retired) as u64));
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_queue_byte_limits_precede_record_limits_and_ordinal_never_wraps() {
    let (queue, statuses) = queue();
    for _ in 0..44 {
        assert!(queue.enqueue(record(&statuses[0], 1476)).is_ok());
    }
    assert!(queue.enqueue(record(&statuses[0], 1476)).is_err());
    for i in 44..177 {
        assert!(queue
            .enqueue(record(&statuses[1 + (i - 44) / 40], 1476))
            .is_ok());
    }
    assert_eq!(queue.resources().1, 177 * 1476);
    assert!(queue.enqueue(record(&statuses[4], 1476)).is_err());
    let before = queue.resources();
    queue.state.lock().unwrap().ordinal = u64::MAX;
    assert!(queue.enqueue(record(&statuses[4], 4)).is_err());
    assert_eq!(queue.resources(), before);
    assert_eq!(queue.state.lock().unwrap().ordinal, u64::MAX);
    drop(queue.close());
    assert_eq!(queue.resources(), (0, 0, 177));
}
