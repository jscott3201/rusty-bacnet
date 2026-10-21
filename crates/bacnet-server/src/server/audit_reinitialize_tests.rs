//! ReinitializeDevice's Audit records (Clause 19.6, Table 19-5, #1564): an
//! accepted request records DEVICE_RESET for WARMSTART and COLDSTART,
//! DEVICE_BACKUP for the backup steps and DEVICE_RESTORE for the restore
//! steps, from the requester with its invoke ID, naming this Device as the
//! target and nothing else. ACTIVATE_CHANGES and every refusal record
//! nothing, and a restart that ends DISABLE_INITIATION records only the
//! reset.
use super::*;
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_types::enums::ReinitializedState;

/// A Reporter that audits the three ReinitializeDevice operations, both
/// DCC operations, and AUDITING_FAILURE.
fn reinitialize_reporter() -> AuditReporterObject {
    let mut reporter = reporter();
    let mut operations = AuditOperationFlags::empty();
    for operation in [
        AuditOperation::DEVICE_RESET,
        AuditOperation::DEVICE_BACKUP,
        AuditOperation::DEVICE_RESTORE,
        AuditOperation::DEVICE_DISABLE_COMM,
        AuditOperation::DEVICE_ENABLE_COMM,
        AuditOperation::AUDITING_FAILURE,
    ] {
        operations.insert(operation);
    }
    reporter.set_auditable_operations(operations).unwrap();
    reporter
}

/// Each state Clause 16.4 defines, with the operation Table 19-5 gives it.
const FAMILIES: [(ReinitializedState, Option<AuditOperation>); 8] = [
    (
        ReinitializedState::COLDSTART,
        Some(AuditOperation::DEVICE_RESET),
    ),
    (
        ReinitializedState::WARMSTART,
        Some(AuditOperation::DEVICE_RESET),
    ),
    (
        ReinitializedState::START_BACKUP,
        Some(AuditOperation::DEVICE_BACKUP),
    ),
    (
        ReinitializedState::END_BACKUP,
        Some(AuditOperation::DEVICE_BACKUP),
    ),
    (
        ReinitializedState::START_RESTORE,
        Some(AuditOperation::DEVICE_RESTORE),
    ),
    (
        ReinitializedState::END_RESTORE,
        Some(AuditOperation::DEVICE_RESTORE),
    ),
    (
        ReinitializedState::ABORT_RESTORE,
        Some(AuditOperation::DEVICE_RESTORE),
    ),
    (ReinitializedState::ACTIVATE_CHANGES, None),
];

fn request(state: ReinitializedState, password: Option<&str>) -> Bytes {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    data.freeze()
}

async fn reinitialize(f: &Fixture, state: ReinitializedState, password: Option<&str>) -> Apdu {
    dispatch(
        &f.server,
        ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        request(state, password),
    )
    .await
}

/// How a test handler answers.
type Answer = fn() -> Result<(), Error>;

fn handle(f: &mut Fixture, outcome: Answer) {
    f.server.config_mut().on_reinitialize = Some(Arc::new(move |_, _| outcome()));
}

/// Check a ReinitializeDevice record: `operation`, from the requester with
/// `invoke_id`, naming Device 10 as its target and nothing else.
fn assert_record(record: &BACnetAuditNotification, operation: AuditOperation, invoke_id: u8) {
    assert_eq!(record.operation, operation);
    assert_eq!(
        record.source_device,
        BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(SOURCE),
        })
    );
    assert_eq!(record.invoke_id, Some(invoke_id));
    assert_eq!(
        record.target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert!(record.target_timestamp.is_some());
    assert_eq!(record.source_timestamp, None);
    assert_eq!(record.source_object, None);
    assert_eq!(record.target_object, None);
    assert_eq!(record.target_property, None);
    assert_eq!(record.target_priority, None);
    assert_eq!(record.target_value, None);
    assert_eq!(record.current_value, None);
    assert_eq!(record.result, None);
}

#[tokio::test(start_paused = true)]
async fn accepted_requests_record_their_table_19_5_operation() {
    for (state, operation) in FAMILIES {
        let mut f = server(reinitialize_reporter()).await;
        handle(&mut f, || Ok(()));
        let Apdu::SimpleAck(ack) = reinitialize(&f, state, None).await else {
            panic!("{state:?} accepted");
        };
        settle().await;
        let frames = audit_frames(&f);
        match operation {
            Some(operation) => {
                assert_eq!(frames.len(), 1, "{state:?}");
                assert_record(&frames[0].1, operation, ack.invoke_id);
            }
            None => assert!(frames.is_empty(), "{state:?} has no Table 19-5 row"),
        }
        f.server.stop().await.unwrap();
    }
}

/// The Reporter's Auditable_Operations still filter the records.
#[tokio::test(start_paused = true)]
async fn records_follow_the_reporters_auditable_operations() {
    let mut f = server(reporter()).await;
    handle(&mut f, || Ok(()));
    for (state, _) in FAMILIES {
        assert!(matches!(
            reinitialize(&f, state, None).await,
            Apdu::SimpleAck(_)
        ));
    }
    settle().await;
    assert!(audit_frames(&f).is_empty());
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn refused_requests_record_nothing() {
    let cases: [(&str, Answer, Option<&str>); 5] = [
        (
            "handler refusal",
            || {
                Err(Error::Protocol {
                    class: ErrorClass::DEVICE,
                    code: ErrorCode::CONFIGURATION_IN_PROGRESS,
                })
            },
            Some("required"),
        ),
        (
            "handler error",
            || Err(Error::Encoding("disk full".into())),
            Some("required"),
        ),
        (
            "handler panic",
            || panic!("handler panic"),
            Some("required"),
        ),
        ("missing password", || Ok(()), None),
        ("wrong password", || Ok(()), Some("wrong")),
    ];
    for (state, _) in FAMILIES {
        for (case, outcome, password) in cases {
            let mut f = server(reinitialize_reporter()).await;
            handle(&mut f, outcome);
            f.server.config_mut().reinit_password = Some("required".into());
            let reply = reinitialize(&f, state, password).await;
            assert!(
                matches!(reply, Apdu::Error(_)),
                "{state:?} {case}: {reply:?}"
            );
            settle().await;
            assert!(audit_frames(&f).is_empty(), "{state:?} {case}");
            f.server.stop().await.unwrap();
        }
    }
    // With no handler every request is refused, and records nothing.
    let mut f = server(reinitialize_reporter()).await;
    for (state, _) in FAMILIES {
        assert!(matches!(
            reinitialize(&f, state, None).await,
            Apdu::Error(_)
        ));
    }
    settle().await;
    assert!(audit_frames(&f).is_empty());
    f.server.stop().await.unwrap();
}

/// Clause 16.1.2 ends DISABLE_INITIATION on an accepted restart. That is
/// part of the reset, so it records DEVICE_RESET alone, with no
/// DEVICE_ENABLE_COMM beside it.
#[tokio::test(start_paused = true)]
async fn a_restart_ending_disable_initiation_records_only_the_reset() {
    for restart in [ReinitializedState::WARMSTART, ReinitializedState::COLDSTART] {
        let mut f = server(reinitialize_reporter()).await;
        handle(&mut f, || Ok(()));
        disable_initiation(&mut f).await;
        settle().await;
        assert_eq!(audit_frames(&f).len(), 1, "the disable's own record");
        let Apdu::SimpleAck(ack) = reinitialize(&f, restart, None).await else {
            panic!("{restart:?} accepted");
        };
        assert_eq!(f.server.comm_state(), DccState::Enable);
        settle().await;
        let frames = audit_frames(&f);
        assert_eq!(frames.len(), 2, "{restart:?}: {frames:?}");
        assert_eq!(frames[0].1.operation, AuditOperation::DEVICE_DISABLE_COMM);
        assert_record(&frames[1].1, AuditOperation::DEVICE_RESET, ack.invoke_id);
        // The cancelled timer owes no expiry record either.
        tokio::time::advance(Duration::from_secs(120)).await;
        settle().await;
        assert_eq!(audit_frames(&f).len(), 2);
        f.server.stop().await.unwrap();
    }
}

/// A reset the application reports for its own reasons is this device's,
/// with no invoke ID.
#[tokio::test(start_paused = true)]
async fn a_reset_the_application_reports_is_recorded_as_this_devices() {
    let mut f = server(reinitialize_reporter()).await;
    f.server.report_device_reset_local().await.unwrap();
    settle().await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1);
    let record = &frames[0].1;
    assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(
        record.source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(record.invoke_id, None);
    assert_eq!(record.target_object, None);
    assert_eq!(record.result, None);
    f.server.stop().await.unwrap();
    assert!(f.server.report_device_reset_local().await.is_err());
}
