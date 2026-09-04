use super::receipt::{
    contains, insert, CompletedAuditReceipt, ConfirmedAuditNotificationOutcome,
    COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS, MAX_COMPLETED_AUDIT_RECEIPTS,
};

fn completed(index: usize, at: u64) -> CompletedAuditReceipt {
    CompletedAuditReceipt::new(index.to_be_bytes().to_vec(), at).unwrap()
}

#[test]
fn retention_boundary_and_future_timestamp_fail_open() {
    let retained = completed(1, 10_000);
    let mut receipts = vec![retained.clone(), completed(2, 80_000)];
    assert!(contains(
        &receipts,
        retained.key(),
        10_000 + COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS - 1
    )
    .unwrap());
    assert!(!contains(
        &receipts,
        retained.key(),
        10_000 + COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS
    )
    .unwrap());
    assert!(!contains(&receipts, completed(2, 0).key(), 70_000).unwrap());

    assert_eq!(
        insert(&mut receipts, completed(3, 70_000)).unwrap(),
        ConfirmedAuditNotificationOutcome::Stored
    );
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].key(), completed(3, 0).key());
}

#[test]
fn full_ledger_evicts_oldest_completion_then_oldest_tie() {
    let mut receipts = (0..MAX_COMPLETED_AUDIT_RECEIPTS)
        .map(|index| completed(index, 100))
        .collect::<Vec<_>>();
    let first_key = receipts[0].key().to_vec();
    let second_key = receipts[1].key().to_vec();

    assert_eq!(
        insert(&mut receipts, completed(MAX_COMPLETED_AUDIT_RECEIPTS, 100)).unwrap(),
        ConfirmedAuditNotificationOutcome::Stored
    );
    assert_eq!(receipts.len(), MAX_COMPLETED_AUDIT_RECEIPTS);
    assert!(!contains(&receipts, &first_key, 100).unwrap());
    assert!(contains(&receipts, &second_key, 100).unwrap());
}
