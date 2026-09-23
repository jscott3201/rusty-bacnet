use super::*;

#[test]
fn endpoint_device_write_claim_stays_within_executed_subset() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows
        .get("BACNET-15-ENDPOINT-DEVICE-WRITE")
        .expect("endpoint write evidence row");
    assert_eq!(row["status"], "in-progress");
    assert!(!row["positive_tests"].as_array().unwrap().is_empty());
    assert!(!row["negative_tests"].as_array().unwrap().is_empty());
    let notes = row["notes"].as_str().unwrap();
    for boundary in [
        "Device.Description",
        "MutationAuthorizer",
        "RP-only",
        "WPM",
        "replay",
        "Audit_Notification_Recipient",
    ] {
        assert!(
            notes.contains(boundary),
            "missing endpoint scope boundary: {boundary}"
        );
    }
}
