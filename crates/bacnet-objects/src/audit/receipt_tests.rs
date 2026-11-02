use super::receipt::{
    COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS, CompletedAuditReceipt,
    ConfirmedAuditNotificationOutcome, MAX_COMPLETED_AUDIT_RECEIPTS, contains, first_repeated_key,
    insert, validate_receipts,
};

fn completed(index: usize, at: u64) -> CompletedAuditReceipt {
    CompletedAuditReceipt::new(index.to_be_bytes().to_vec(), at).unwrap()
}

#[test]
fn retention_boundary_and_future_timestamp_fail_open() {
    let retained = completed(1, 10_000);
    let mut receipts = vec![retained.clone(), completed(2, 80_000)];
    assert!(
        contains(
            &receipts,
            retained.key(),
            10_000 + COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS - 1
        )
        .unwrap()
    );
    assert!(
        !contains(
            &receipts,
            retained.key(),
            10_000 + COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS
        )
        .unwrap()
    );
    assert!(!contains(&receipts, completed(2, 0).key(), 70_000).unwrap());

    assert_eq!(
        insert(&mut receipts, completed(3, 70_000)).unwrap(),
        ConfirmedAuditNotificationOutcome::Stored
    );
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].key(), completed(3, 0).key());
}

#[test]
fn full_receipt_list_evicts_oldest_completion_then_oldest_tie() {
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

#[test]
fn a_repeated_key_is_refused_wherever_it_sits() {
    assert!(validate_receipts(&[]).is_ok());
    assert!(validate_receipts(&[completed(1, 0)]).is_ok());
    let distinct: Vec<_> = (0..40).rev().map(|index| completed(index, 0)).collect();
    assert!(validate_receipts(&distinct).is_ok());
    for (first, second) in [(0, 1), (0, 39), (17, 18), (5, 30), (38, 39)] {
        let mut receipts = distinct.clone();
        // Another time, the same key: only the key counts.
        receipts[second] = CompletedAuditReceipt::new(receipts[first].key().to_vec(), 9).unwrap();
        let error = validate_receipts(&receipts).unwrap_err();
        assert!(
            format!("{error:?}").contains("must be unique"),
            "{first}, {second}: {error:?}"
        );
    }
}

#[test]
fn the_first_repeat_is_the_earliest_index_that_repeats_a_key() {
    // Keys by position: a b c c b d a e d. The repeats sit at 3 (c), 4 (b),
    // 6 (a) and 8 (d); the earliest is 3, though `a` sorts first.
    let receipts: Vec<_> = [1, 2, 3, 3, 2, 4, 1, 5, 4]
        .into_iter()
        .map(|key| completed(key, 0))
        .collect();
    assert_eq!(first_repeated_key(&receipts), Some(3));
    assert_eq!(first_repeated_key(&receipts[..3]), None);
    // From b: b d a e d, where only d repeats, at 4.
    assert_eq!(first_repeated_key(&receipts[4..]), Some(4));
}
