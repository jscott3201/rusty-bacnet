use super::*;

#[tokio::test(start_paused = true)]
async fn cov_order_supplied_terminal_snapshot_orders_preparation_not_object_age() {
    let (mut server, oid, sent) = start_server(2).await;
    if let Some(task) = server.binary_lighting_operation_task.take() {
        task.abort();
        let _ = task.await;
    }
    let sub = server
        .cov_table
        .write()
        .await
        .admit_for_test(
            CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xba, 0xd0]),
                subscriber_network: None,
                subscriber_process_identifier: 7,
                monitored_object_identifier: oid,
                issue_confirmed_notifications: false,
                expires_at: None,
                last_notified_observation: None,
                monitored_property: None,
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            },
            0,
        )
        .unwrap();
    let old_snapshot = {
        let mut db = server.db.write().await;
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(3),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
        assert!(
            obj.advance_monotonic_time_internal(obj.next_monotonic_deadline_internal().unwrap())
        );
        obj.cov_snapshot_internal().unwrap()
    };
    // The newer live command is observed and successfully sent first.
    write_command(&server, oid, 1, 4).await;
    assert_eq!(
        server
            .cov_table
            .read()
            .await
            .get_subscription(sub.key())
            .unwrap()
            .last_notified_observation
            .as_ref()
            .unwrap()
            .sample()
            .value(),
        &PropertyValue::Enumerated(1)
    );
    // The intentionally retained older terminal snapshot is prepared later;
    // its new ticket may replace the newer object's previously sent baseline.
    BACnetServer::<TestTransport>::fire_cov_notifications_from_snapshot(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &server.db,
            network: server.test_network(),
            cov_table: &server.cov_table,
            cov_in_flight: &server.cov_in_flight,
            notification_transactions: &server.notification_transactions,
            comm_state: &server.comm_state,
            config: &server.config,
        },
        &oid,
        old_snapshot.as_ref(),
    )
    .await;
    assert_eq!(
        server
            .cov_table
            .read()
            .await
            .get_subscription(sub.key())
            .unwrap()
            .last_notified_observation
            .as_ref()
            .unwrap()
            .sample()
            .value(),
        &PropertyValue::Enumerated(0)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Enumerated(1),
        "live object remains newer"
    );
    let values = sent
        .lock()
        .iter()
        .map(|frame| {
            let Apdu::UnconfirmedRequest(request) =
                decode_apdu(decode_npdu(frame.npdu.clone()).unwrap().payload).unwrap()
            else {
                panic!()
            };
            let n = COVNotificationRequest::decode(&request.service_request).unwrap();
            let bytes = &n
                .list_of_values
                .iter()
                .find(|v| v.property_identifier == PropertyIdentifier::PRESENT_VALUE)
                .unwrap()
                .value;
            bacnet_encoding::primitives::decode_application_value(bytes, 0)
                .unwrap()
                .0
        })
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        [PropertyValue::Enumerated(1), PropertyValue::Enumerated(0)]
    );
    server.stop().await.unwrap();
}
