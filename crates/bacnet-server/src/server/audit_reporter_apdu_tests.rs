use super::*;

#[tokio::test(start_paused = true)]
async fn immediate_and_delayed_reporter_raw_1474_emit_header_1024() {
    for delayed in [false, true] {
        let mut reporter = if delayed {
            batching::delayed(1)
        } else {
            reporter()
        };
        reporter.set_issue_confirmed_notifications(true).unwrap();
        let mut f = try_servers_config(
            vec![reporter],
            &[1],
            Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
            vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
            true,
            1474,
            AuditCapture::default(),
        )
        .await
        .unwrap();
        assert!(matches!(
            write_value(&f.server, None).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        if delayed {
            assert!(f.transport.sent.lock().unwrap().is_empty());
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
        }
        let request = confirmed_notification(&f.transport.sent, 0);
        assert_eq!(request.max_apdu_length, 1024);
        assert!(f.server.notification_transactions.admit_terminal(
            LOGGER,
            None,
            None,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice
            })
        ));
        f.server.stop().await.unwrap();
    }
}
