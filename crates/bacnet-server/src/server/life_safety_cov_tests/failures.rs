use super::*;

use bacnet_objects::life_safety::{
    LifeSafetyPointResetContext, LifeSafetyPointResetExecutor, LifeSafetyResetError,
};

fn failed_reset(
    _: &LifeSafetyPointResetContext,
) -> Result<LifeSafetyPointResetCommit, LifeSafetyResetError> {
    Err(LifeSafetyResetError::ServiceRequestDenied)
}

fn invalid_reset(
    _: &LifeSafetyPointResetContext,
) -> Result<LifeSafetyPointResetCommit, LifeSafetyResetError> {
    Ok(LifeSafetyPointResetCommit {
        present_value: Some(bacnet_types::enums::LifeSafetyState::from_raw(100)),
        ..Default::default()
    })
}

fn operation_bytes(operation: LifeSafetyOperation) -> Bytes {
    let request = LifeSafetyOperationRequest {
        requesting_process_identifier: 20,
        requesting_source: "operator".into(),
        request: operation,
        object_identifier: Some(point_oid()),
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    encoded.freeze()
}

#[tokio::test]
async fn denied_invalid_and_panicking_operations_emit_no_cov() {
    let mut denied_point = LifeSafetyPointObject::new(1, "denied").unwrap();
    denied_point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut denied_db = clocked_test_database();
    denied_db.add(Box::new(denied_point)).unwrap();
    let mut denied = DispatchFixture::new(
        denied_db,
        [subscription(
            Some(PropertyIdentifier::SILENCED),
            CovNotificationKind::Single,
            1,
        )],
    )
    .await;
    denied.config.life_safety_operation_authorizer = None;
    denied
        .dispatch(
            1,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            operation_bytes(LifeSafetyOperation::SILENCE),
        )
        .await;
    assert!(matches!(denied.take_apdus().as_slice(), [Apdu::Error(_)]));

    let invalid = DispatchFixture::new(
        life_safety_db(),
        [subscription(
            Some(PropertyIdentifier::SILENCED),
            CovNotificationKind::Single,
            2,
        )],
    )
    .await;
    invalid
        .dispatch(
            2,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            operation_bytes(LifeSafetyOperation::SILENCE),
        )
        .await;
    assert!(matches!(invalid.take_apdus().as_slice(), [Apdu::Error(_)]));

    let mut panic_point = LifeSafetyPointObject::new(1, "panic").unwrap();
    panic_point.set_operation_expected(LifeSafetyOperation::RESET);
    panic_point.set_reset_executor(Arc::new(|_| panic!("executor bug")));
    let mut panic_db = clocked_test_database();
    panic_db.add(Box::new(panic_point)).unwrap();
    let panicking = DispatchFixture::new(
        panic_db,
        [subscription(None, CovNotificationKind::Single, 3)],
    )
    .await;
    panicking
        .dispatch(
            3,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            operation_bytes(LifeSafetyOperation::RESET),
        )
        .await;
    assert!(matches!(
        panicking.take_apdus().as_slice(),
        [Apdu::Error(_)]
    ));

    for (invoke_id, executor) in [
        (4, Arc::new(failed_reset) as LifeSafetyPointResetExecutor),
        (5, Arc::new(invalid_reset) as LifeSafetyPointResetExecutor),
    ] {
        let mut point = LifeSafetyPointObject::new(1, "failed-reset").unwrap();
        point.set_operation_expected(LifeSafetyOperation::RESET);
        point.set_reset_executor(executor);
        let mut db = clocked_test_database();
        db.add(Box::new(point)).unwrap();
        let fixture = DispatchFixture::new(
            db,
            [subscription(
                None,
                CovNotificationKind::Single,
                invoke_id.into(),
            )],
        )
        .await;
        fixture
            .dispatch(
                invoke_id,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
                operation_bytes(LifeSafetyOperation::RESET),
            )
            .await;
        assert!(matches!(fixture.take_apdus().as_slice(), [Apdu::Error(_)]));
    }
}
