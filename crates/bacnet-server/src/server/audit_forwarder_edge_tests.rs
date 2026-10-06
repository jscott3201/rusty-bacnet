use super::*;

#[tokio::test(start_paused = true)]
async fn audit_forwarding_expired_before_first_poll_never_sends() {
    let mut f = ready().await;
    f.unconfirmed(payload(false)).await;
    // Advance the clock before the spawned delivery gets its first poll.
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert!(f.requests().is_empty());
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_routed_ack_uses_final_peer_not_router() {
    let mut f = fixture(
        Some(parent()),
        Some(DeviceBinding::routed(oid(ObjectType::DEVICE, 20), 200, [9], [2]).unwrap()),
    )
    .await;
    f.unconfirmed(payload(false)).await;
    settle().await;
    let wire = f.wire.sent.lock().unwrap()[0].clone();
    let npdu = decode_npdu(wire).unwrap();
    assert_eq!(
        npdu.destination,
        Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[9])
        })
    );
    let req = f.requests().remove(0);
    assert!(!f.ack(req.invoke_id, &[2], req.service_choice));
    assert!(f.server.notification_transactions.admit_terminal(
        &[2],
        Some(&NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[9])
        }),
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: req.invoke_id,
            service_choice: req.service_choice
        })
    ));
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    f.server.stop().await.unwrap();
}

/// A parent bound at `[2]` on network 200 behind router `[9]`, on a server
/// whose own network is numbered 200, gets its copy straight at `[2]` with
/// no DNET, and its ACK from there, with no SNET, completes it (#1358). The
/// link accepts unicasts to `[2]` only, so nothing went to the router.
#[tokio::test(start_paused = true)]
async fn audit_forwarding_to_a_parent_routed_through_this_network_goes_without_a_dnet() {
    let mut f = fixture(
        Some(parent()),
        Some(DeviceBinding::routed(oid(ObjectType::DEVICE, 20), 200, [2], [9]).unwrap()),
    )
    .await;
    f.server
        .test_network()
        .local_network_number()
        .publish(bacnet_types::network_number::NetworkNumber::configured(200).unwrap());
    f.unconfirmed(payload(false)).await;
    settle().await;
    let wire = f.wire.sent.lock().unwrap()[0].clone();
    assert_eq!(decode_npdu(wire).unwrap().destination, None);
    let req = f.requests().remove(0);
    assert!(f.ack(req.invoke_id, &[2], req.service_choice));
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    f.server.stop().await.unwrap();
}

/// #1493: a parent bound at a group address of the link, as its own MAC or
/// its router's, names no device. Forwarding finds no route, so no copy goes
/// out and the log reports a configuration error.
#[tokio::test(start_paused = true)]
async fn audit_forwarding_to_a_parent_bound_at_a_group_address_never_sends() {
    let parent_device = oid(ObjectType::DEVICE, 20);
    for binding in [
        DeviceBinding::local(parent_device, GROUP),
        DeviceBinding::routed(parent_device, 200, [9], GROUP),
    ] {
        let mut f = fixture(Some(parent()), None).await;
        // Inserted past the link's check, which no started server skips.
        f.server
            .device_bindings
            .write()
            .await
            .insert_configured(binding.unwrap(), |_| false)
            .unwrap();
        f.unconfirmed(payload(false)).await;
        settle().await;
        assert!(f.requests().is_empty());
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        f.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_observed_binding_and_local_alias_do_not_send() {
    let mut f = fixture(Some(parent()), None).await;
    f.server.device_bindings.write().await.observe_i_am_at(
        oid(ObjectType::DEVICE, 20),
        &[2],
        None,
        runtime_clock::now(),
        |_| false,
    );
    f.unconfirmed(payload(false)).await;
    settle().await;
    assert!(f.requests().is_empty());
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    f.server.stop().await.unwrap();
    let mut f = fixture(
        Some(parent()),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [1]).unwrap()),
    )
    .await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    f.unconfirmed(payload(false)).await;
    settle().await;
    assert!(f.requests().is_empty());
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_denial_disabled_and_malformed_batches_are_silent() {
    let mut f = ready().await;
    f.server.config_mut().audit_notification_authorizer = Some(Arc::new(|_| false));
    f.server
        .config_mut()
        .unconfirmed_audit_notification_authorizer = Some(Arc::new(|_| false));
    assert!(matches!(
        f.confirmed(1, &[3], payload(false)).await,
        Some(Apdu::Error(_))
    ));
    f.unconfirmed(payload(false)).await;
    f.server.config_mut().audit_notification_authorizer = Some(Arc::new(|_| true));
    f.server
        .config_mut()
        .unconfirmed_audit_notification_authorizer = Some(Arc::new(|_| true));
    let mut malformed = payload(false).to_vec();
    malformed.pop();
    assert!(matches!(
        f.confirmed(2, &[3], malformed.clone().into()).await,
        Some(Apdu::Reject(_))
    ));
    f.unconfirmed(malformed.into()).await;
    f.server
        .db
        .write()
        .await
        .get_mut(&oid(ObjectType::AUDIT_LOG, 7))
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    let before = f.store.snapshot.lock().unwrap().clone();
    assert!(matches!(
        f.confirmed(3, &[3], payload(false)).await,
        Some(Apdu::Error(_))
    ));
    f.unconfirmed(payload(false)).await;
    settle().await;
    assert!(f.requests().is_empty());
    assert_eq!(*f.store.snapshot.lock().unwrap(), before);
    f.server.stop().await.unwrap();
}

/// Put the server under DISABLE_INITIATION for one minute through a wire
/// request from `[3]`, so the DCC timer is live.
async fn disable_initiation(f: &mut Fixture) {
    use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: Some(1),
        enable_disable: bacnet_types::enums::EnableDisable::DISABLE_INITIATION,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    let mut request = confirmed_request(90, data.freeze());
    request.service_choice = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
    let s = &f.server;
    let (tx, rx) = oneshot::channel();
    BACnetServer::handle_confirmed_request(
        &s.test_services(),
        &s.confirmed_request_tracker,
        &s.request_tasks.spawner(),
        &[3],
        None,
        request,
        Some(tx),
    )
    .await;
    let response = decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);
}

/// A forward is an audit notification, which Clause 16.1 leaves running under
/// DISABLE_INITIATION: both receipt paths forward, the ACK keeps the sink
/// healthy, and the DCC timer re-enabling initiation forwards nothing again.
#[tokio::test(start_paused = true)]
async fn audit_forwarding_goes_out_under_disable_initiation() {
    let mut f = ready().await;
    disable_initiation(&mut f).await;
    assert!(matches!(
        f.confirmed(1, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    f.unconfirmed(payload(false)).await;
    settle().await;
    let requests = f.requests();
    assert_eq!(requests.len(), 2, "both receipts forward under DCC");
    for request in &requests {
        assert!(f.ack(request.invoke_id, &[2], request.service_choice));
    }
    settle().await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    tokio::time::advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(
        f.server.comm_state(),
        DccState::Enable,
        "the DCC timer re-enables"
    );
    assert_eq!(f.requests().len(), 2, "nothing is forwarded again");
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_oversize_and_send_errors_leave_local_success() {
    let mut f = ready().await;
    f.server.config_mut().max_apdu_length = 50;
    assert!(matches!(
        f.confirmed(2, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    assert!(
        f.requests().is_empty(),
        "oversized forward is not segmented"
    );
    f.server.config_mut().max_apdu_length = 1476;
    f.wire.fail.store(true, Ordering::Release);
    assert!(matches!(
        f.confirmed(3, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(f.requests().len(), 1);
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    assert_eq!(
        f.store
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completed_receipts
            .len(),
        2
    );
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_terminal_failures_and_late_success_cannot_hide_new_failure() {
    let mut f = ready().await;
    f.unconfirmed(payload(false)).await;
    f.unconfirmed(payload(false)).await;
    settle().await;
    let requests = f.requests();
    let failure = Apdu::Reject(RejectPdu {
        invoke_id: requests[1].invoke_id,
        reject_reason: RejectReason::OTHER,
    });
    assert!(f
        .server
        .notification_transactions
        .admit_terminal(&[2], None, None, &failure));
    settle().await;
    assert!(f.ack(requests[0].invoke_id, &[2], requests[0].service_choice));
    settle().await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    for abort in [false, true] {
        f.unconfirmed(payload(false)).await;
        settle().await;
        let req = f.requests().pop().unwrap();
        let terminal = if abort {
            // The recipient serves the notification, so its Abort is server-flagged.
            Apdu::Abort(AbortPdu {
                invoke_id: req.invoke_id,
                sent_by_server: true,
                abort_reason: AbortReason::OTHER,
            })
        } else {
            Apdu::Error(ErrorPdu {
                invoke_id: req.invoke_id,
                service_choice: req.service_choice,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::SERVICE_REQUEST_DENIED,
                error_data: Bytes::new(),
            })
        };
        assert!(f
            .server
            .notification_transactions
            .admit_terminal(&[2], None, None, &terminal));
        settle().await;
        assert_eq!(f.server.notification_transactions.active_count(), 0);
    }
    assert_eq!(f.requests().len(), 4);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_reopen_keeps_receipt_but_never_replays_forwarding_progress() {
    let mut f = ready().await;
    f.wire.block.store(true, Ordering::Release);
    let data = payload(false);
    assert!(matches!(
        f.confirmed(201, &[3], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    f.server.stop().await.unwrap();
    let snapshot = f.store.snapshot.lock().unwrap().clone();
    let mut reopened = fixture_with(
        10,
        Some(parent()),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
        f.store.clone(),
    )
    .await;
    settle().await;
    assert!(reopened.requests().is_empty());
    assert!(reopened.confirmed(201, &[3], data).await.is_none());
    assert_eq!(*reopened.store.snapshot.lock().unwrap(), snapshot);
    assert!(reopened.requests().is_empty());
    reopened.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_isolated_cycle_is_content_bounded_not_global_deduplication() {
    let mut a = ready().await;
    let mut b_parent = parent();
    b_parent.device_identifier = Some(oid(ObjectType::DEVICE, 10));
    let mut b = fixture_with(
        20,
        Some(b_parent),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 10), [2]).unwrap()),
        Arc::new(MemoryPersistence::default()),
    )
    .await;
    a.unconfirmed(payload(true)).await;
    settle().await;
    let outgoing = a.requests()[0].clone();
    assert!(matches!(
        b.confirmed(outgoing.invoke_id, &[2], outgoing.service_request.clone())
            .await,
        Some(Apdu::SimpleAck(_))
    ));
    assert!(a.ack(outgoing.invoke_id, &[2], outgoing.service_choice));
    settle().await;
    let returning = b.requests()[0].clone();
    assert!(matches!(
        a.confirmed(returning.invoke_id, &[2], returning.service_request)
            .await,
        Some(Apdu::SimpleAck(_))
    ));
    assert!(b.ack(returning.invoke_id, &[2], returning.service_choice));
    settle().await;
    assert_eq!(a.requests().len(), 1);
    assert_eq!(b.requests().len(), 1);
    assert_eq!(a.server.notification_transactions.active_count(), 0);
    assert_eq!(b.server.notification_transactions.active_count(), 0);
    // Single-actor content is not a complementary match. There is deliberately
    // no cross-peer/hop dedupe claim: bounded overload/deadline tests cover that
    // case, and applications must configure an acyclic hierarchy.
    a.server.stop().await.unwrap();
    b.server.stop().await.unwrap();
}

/// With several Devices the lowest is this device (#1204). Receipt reads its
/// APDU_Timeout, and a parent naming any other Device is forwarded, even one
/// this database also holds; a parent naming the lowest is this device and
/// isn't. A database whose only Device is the wildcard has no identity to tell
/// a parent from itself, so it forwards nothing.
#[tokio::test(start_paused = true)]
async fn audit_forwarding_follows_the_lowest_of_several_devices() {
    let wildcard = ObjectIdentifier::WILDCARD_INSTANCE;
    for (devices, parent_device, forwards) in [
        (&[30, 10][..], 20, true),
        (&[10, 20][..], 20, true),
        (&[30, 10][..], 10, false),
        (&[wildcard][..], 20, false),
    ] {
        let store = Arc::new(MemoryPersistence::default());
        let mut log = AuditLogObject::new(7, "forwarder", 16, store.clone()).unwrap();
        log.set_member_of(Some(BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, parent_device)),
            ..parent()
        }));
        let binding = DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap();
        let (server, wire) = start_on(devices, log, Some(binding), 1476).await;
        let mut f = Fixture {
            server,
            store,
            wire,
        };
        let case = format!("{devices:?} naming Device {parent_device}");
        assert!(
            matches!(
                f.confirmed(201, &[3], payload(false)).await,
                Some(Apdu::SimpleAck(_))
            ),
            "{case}"
        );
        settle().await;
        assert_eq!(f.requests().len(), usize::from(forwards), "{case}");
        let expected = if forwards {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::CONFIGURATION_ERROR
        };
        if forwards {
            let request = f.requests().remove(0);
            assert!(f.ack(request.invoke_id, &[2], request.service_choice));
            settle().await;
        }
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(expected.to_raw()),
            "{case}"
        );
        f.server.stop().await.unwrap();
    }
}
