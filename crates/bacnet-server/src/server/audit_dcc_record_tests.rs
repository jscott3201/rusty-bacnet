//! DeviceCommunicationControl's own Audit records (Clause 19.6, Table 19-5,
//! #1387): DEVICE_DISABLE_COMM when DISABLE_INITIATION is carried out,
//! DEVICE_ENABLE_COMM when ENABLE is, both from the requester with its
//! invoke ID, and DEVICE_ENABLE_COMM from this device when a timed disable
//! runs out. They name this Device as the target and no object, property or
//! value. A refused request writes none.
use super::*;
use crate::server::dcc_disable_rate::DccDisableRateLimit;
use crate::server::request_tasks::RequestTasks;

/// A Reporter that audits both DCC operations, and AUDITING_FAILURE.
fn dcc_reporter() -> AuditReporterObject {
    let mut reporter = reporter();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::DEVICE_DISABLE_COMM);
    operations.insert(AuditOperation::DEVICE_ENABLE_COMM);
    operations.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(operations).unwrap();
    reporter
}

fn request(mode: EnableDisable, minutes: Option<u16>, password: Option<&str>) -> Bytes {
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: minutes,
        enable_disable: mode,
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    data.freeze()
}

/// Send a DCC request from `SOURCE`, as a peer would.
async fn dcc(
    f: &Fixture,
    mode: EnableDisable,
    minutes: Option<u16>,
    password: Option<&str>,
) -> Apdu {
    dispatch(
        &f.server,
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        request(mode, minutes, password),
    )
    .await
}

/// The invoke ID of a request the server carried out.
fn carried_out(response: Apdu) -> u8 {
    match response {
        Apdu::SimpleAck(ack) => ack.invoke_id,
        other => panic!("expected a SimpleACK, got {other:?}"),
    }
}

/// Check a DCC record: `operation`, from `source` with `invoke_id`, naming
/// Device 10 as its target and nothing else.
fn assert_dcc_record(
    record: &BACnetAuditNotification,
    operation: AuditOperation,
    source: BACnetRecipient,
    invoke_id: Option<u8>,
) {
    assert_eq!(record.operation, operation);
    assert_eq!(record.source_device, source);
    assert_eq!(record.invoke_id, invoke_id);
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

fn requester() -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(SOURCE),
    })
}

fn this_device() -> BACnetRecipient {
    BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
}

#[tokio::test(start_paused = true)]
async fn disable_initiation_and_enable_are_audited_with_their_requester() {
    let mut f = server(dcc_reporter()).await;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    let disable = carried_out(dcc(&f, EnableDisable::DISABLE_INITIATION, Some(5), None).await);
    settle().await;
    // The record goes out under the state it reports.
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1);
    assert_dcc_record(
        &frames[0].1,
        AuditOperation::DEVICE_DISABLE_COMM,
        requester(),
        Some(disable),
    );
    let enable = carried_out(dcc(&f, EnableDisable::ENABLE, None, None).await);
    settle().await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 2);
    assert_dcc_record(
        &frames[1].1,
        AuditOperation::DEVICE_ENABLE_COMM,
        requester(),
        Some(enable),
    );
    // ENABLE cancelled the disable's timer: its expiry owes nothing.
    tokio::time::advance(Duration::from_secs(5 * 60)).await;
    settle().await;
    assert_eq!(audit_frames(&f).len(), 2);
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_timed_disable_running_out_is_audited_as_this_devices_enable() {
    let mut f = server(dcc_reporter()).await;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    carried_out(dcc(&f, EnableDisable::DISABLE_INITIATION, Some(1), None).await);
    settle().await;
    assert_eq!(audit_frames(&f).len(), 1);
    expire_dcc(&f).await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 2, "the expiry is reported once");
    // No request caused it: this device is its source, with no invoke ID.
    assert_dcc_record(
        &frames[1].1,
        AuditOperation::DEVICE_ENABLE_COMM,
        this_device(),
        None,
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_enables_own_timer_reports_nothing_more() {
    let mut f = server(dcc_reporter()).await;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    let enable = carried_out(dcc(&f, EnableDisable::ENABLE, Some(1), None).await);
    tokio::time::advance(Duration::from_secs(61)).await;
    settle().await;
    let timer = f.server.dcc_timer.lock().await;
    assert!(
        timer.as_ref().is_some_and(JoinHandle::is_finished),
        "the ENABLE's timer ran out"
    );
    drop(timer);
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1, "ENABLE's timer changes nothing");
    assert_dcc_record(
        &frames[0].1,
        AuditOperation::DEVICE_ENABLE_COMM,
        requester(),
        Some(enable),
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn refused_dcc_requests_write_no_record() {
    let mut f = server(dcc_reporter()).await;
    let refused = |response: Apdu, case: &str| {
        assert!(matches!(response, Apdu::Error(_)), "{case}: {response:?}");
    };
    // Policy: the default denies every mode.
    refused(dcc(&f, EnableDisable::ENABLE, None, None).await, "deny all");
    f.server.config_mut().dcc_policy = DccPolicy::RequirePassword;
    f.server.config_mut().dcc_password = Some("required".into());
    refused(
        dcc(&f, EnableDisable::DISABLE_INITIATION, None, Some("wrong")).await,
        "password",
    );
    refused(
        dcc(&f, EnableDisable::DISABLE, None, Some("required")).await,
        "DISABLE",
    );
    f.server.config_mut().dcc_source_restriction =
        Some(crate::server::DccSourceRestriction::new(vec![]).unwrap());
    refused(
        dcc(&f, EnableDisable::ENABLE, None, Some("required")).await,
        "source",
    );
    f.server.config_mut().dcc_source_restriction = None;
    // Rate: a one-token bucket refuses the second DISABLE_INITIATION.
    let tasks = RequestTasks::for_server(&ServerConfig {
        dcc_disable_rate_limit: Some(DccDisableRateLimit {
            capacity: 1,
            refill_interval_ms: 86_400_000,
        }),
        ..Default::default()
    })
    .unwrap();
    let mut answers = Vec::new();
    for invoke_id in [1, 2] {
        let (tx, rx) = oneshot::channel();
        BACnetServer::handle_confirmed_request(
            &f.server.test_services(),
            &f.server.confirmed_request_tracker,
            &tasks.spawner(),
            SOURCE,
            None,
            ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                service_request: request(EnableDisable::DISABLE_INITIATION, None, Some("required")),
            },
            Some(tx),
        )
        .await;
        answers.push(decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap());
    }
    settle().await;
    // The first was carried out and reported; the refused second was not.
    assert!(matches!(answers[0], Apdu::SimpleAck(_)));
    refused(answers.remove(1), "rate");
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1, "only the request carried out");
    assert_eq!(frames[0].1.operation, AuditOperation::DEVICE_DISABLE_COMM);
    assert_eq!(frames[0].1.invoke_id, Some(1));
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dcc_records_follow_the_reporters_operations_delay_and_confirmation() {
    // A Reporter that audits neither DCC operation writes no DCC record.
    let mut f = server(reporter()).await;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    carried_out(dcc(&f, EnableDisable::DISABLE_INITIATION, Some(1), None).await);
    expire_dcc(&f).await;
    assert!(audit_frames(&f).is_empty());
    f.server.stop().await.unwrap();

    // A delayed Reporter holds the record for its Maximum_Send_Delay; a
    // confirmed one sends it confirmed, and the ACK keeps it healthy.
    let mut reporter = batching::delayed(1);
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::DEVICE_DISABLE_COMM);
    reporter.set_auditable_operations(operations).unwrap();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut f = server(reporter).await;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    carried_out(dcc(&f, EnableDisable::DISABLE_INITIATION, None, None).await);
    settle().await;
    assert!(audit_frames(&f).is_empty(), "waits for its send delay");
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1.operation, AuditOperation::DEVICE_DISABLE_COMM);
    ack(&f, frames[0].0.expect("confirmed"));
    settle().await;
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}

/// A DCC record dropped for want of a send slot is a resource loss like any
/// other, summarized as AUDITING_FAILURE.
#[tokio::test(start_paused = true)]
async fn a_dropped_dcc_record_is_summarized() {
    let mut f = server(dcc_reporter()).await;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    let permits: Vec<_> = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    carried_out(dcc(&f, EnableDisable::DISABLE_INITIATION, None, None).await);
    settle().await;
    assert!(audit_frames(&f).is_empty());
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (true, 1, 0)
    );
    drop(permits);
    settle().await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1.operation, AuditOperation::AUDITING_FAILURE);
    assert_eq!(frames[0].1.current_value, Some(vec![0x21, 1]));
    f.server.stop().await.unwrap();
}
