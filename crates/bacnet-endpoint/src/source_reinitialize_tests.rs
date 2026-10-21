//! ReinitializeDevice Audit records on an endpoint session (Clause 19.6,
//! Table 19-5, #1564): the session's Reporter records an accepted reset,
//! backup or restore step from the requester, and nothing for
//! ACTIVATE_CHANGES or any refusal.
use super::*;
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::ReinitializedState;
use std::sync::atomic::{AtomicU8, Ordering};

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
    (ReinitializedState::ACTIVATE_CHANGES, None),
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
];

/// How the handler answers: accept, refuse, panic, or fail.
const ACCEPT: u8 = 0;
const REFUSE: u8 = 1;
const PANIC: u8 = 2;
const FAIL: u8 = 3;

/// The session's own address, learned from a frame it sends `peer`.
async fn session_mac(
    session: &EndpointSession<BipTransport>,
    peer: &NetworkLayer<BipTransport>,
    replies: &mut bacnet_network::layer::ApduReceiver,
) -> MacAddr {
    session
        .egress
        .as_ref()
        .unwrap()
        .send_apdu(
            vec![0x10, 8],
            bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(peer.local_mac()),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
        )
        .await
        .unwrap();
    receive(replies).await.source_mac
}

async fn reinitialize(
    peer: &NetworkLayer<BipTransport>,
    session: &[u8],
    replies: &mut bacnet_network::layer::ApduReceiver,
    state: ReinitializedState,
    password: Option<&str>,
    invoke_id: u8,
) -> Apdu {
    let mut service = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: password.map(str::to_owned),
    }
    .encode(&mut service)
    .unwrap();
    send(
        peer,
        session,
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
            service_request: service.freeze(),
        }),
    )
    .await;
    decode_apdu(receive(replies).await.apdu).unwrap()
}

/// Every refusal kind, then the accepted request, for each state family.
/// The records carry consecutive sequence-number timestamps, so a record
/// built for a refusal or for ACTIVATE_CHANGES would show as a gap.
#[tokio::test]
async fn endpoint_records_accepted_reinitialize_requests_only() {
    let (mut sink, mut records) = network().await;
    let (mut peer, mut replies) = network().await;
    let mut db = database(false);
    db.set_clock_reader(None);
    let mut operations = AuditOperationFlags::empty();
    for operation in [
        AuditOperation::DEVICE_RESET,
        AuditOperation::DEVICE_BACKUP,
        AuditOperation::DEVICE_RESTORE,
    ] {
        operations.insert(operation);
    }
    db.get_mut(&selected())
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            operations,
            false,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
    let mode = Arc::new(AtomicU8::new(ACCEPT));
    let answer = Arc::clone(&mode);
    let mut session = session(db, SessionRole::Both, &sink)
        .with_reinitialize(move |_, _| match answer.load(Ordering::SeqCst) {
            ACCEPT => Ok(()),
            REFUSE => Err(Error::Protocol {
                class: ErrorClass::DEVICE,
                code: ErrorCode::CONFIGURATION_IN_PROGRESS,
            }),
            PANIC => panic!("reinitialize handler panic"),
            _ => Err(Error::Encoding("disk full".into())),
        })
        .with_reinit_password("pw");
    session.start().await.unwrap();
    let mac = session_mac(&session, &peer, &mut replies).await;
    let requester = BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(peer.local_mac()),
    });
    let (mut invoke, mut sequence) = (0u8, 0u16);
    for (state, operation) in FAMILIES {
        for (answer, password) in [
            (REFUSE, Some("pw")),
            (PANIC, Some("pw")),
            (FAIL, Some("pw")),
            (ACCEPT, None),
            (ACCEPT, Some("wrong")),
        ] {
            mode.store(answer, Ordering::SeqCst);
            invoke += 1;
            let reply = reinitialize(&peer, &mac, &mut replies, state, password, invoke).await;
            assert!(matches!(reply, Apdu::Error(_)), "{state:?}: {reply:?}");
        }
        mode.store(ACCEPT, Ordering::SeqCst);
        invoke += 1;
        let reply = reinitialize(&peer, &mac, &mut replies, state, Some("pw"), invoke).await;
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{state:?}: {reply:?}");
        let Some(operation) = operation else {
            continue;
        };
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.operation, operation, "{state:?}");
        assert_eq!(record.source_device, requester);
        assert_eq!(record.invoke_id, Some(invoke));
        assert_eq!(
            record.target_device,
            BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
        );
        assert_eq!(
            record.target_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(sequence)),
            "{state:?}: no record before this one since the last"
        );
        assert_eq!(record.source_timestamp, None);
        assert_eq!(record.target_object, None);
        assert_eq!(record.target_property, None);
        assert_eq!(record.target_value, None);
        assert_eq!(record.result, None);
        sequence += 1;
    }
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
    peer.stop().await.unwrap();
}
