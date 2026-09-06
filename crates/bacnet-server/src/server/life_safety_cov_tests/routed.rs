use super::*;

#[tokio::test]
async fn exact_confirmed_multiple_preserves_routed_peer_and_mode() {
    let router = MacAddr::from_slice(&[192, 168, 1, 1, 0xBA, 0xC0]);
    let remote = NpduAddress {
        network: 222,
        mac_address: MacAddr::from_slice(&[9, 8, 7]),
    };
    let mut sub = subscription(
        Some(PropertyIdentifier::SILENCED),
        CovNotificationKind::Multiple,
        14,
    );
    sub.subscriber_mac = router.clone();
    sub.subscriber_network = Some(remote.clone());
    sub.issue_confirmed_notifications = true;
    let fixture = ExactFixture::new([sub]).await;

    fixture.fire(&[PropertyIdentifier::SILENCED]).await;
    for _ in 0..32 {
        if !fixture.sent.lock().unwrap().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    let sent = fixture.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, router);
    let npdu = decode_npdu(sent[0].0.clone()).unwrap();
    assert_eq!(npdu.destination, Some(remote.clone()));
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected confirmed COVNotificationMultiple");
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
    );
    let invoke_id = request.invoke_id;
    drop(sent);
    assert!(fixture.transactions.admit_terminal(
        &router,
        Some(&remote),
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        }),
    ));
}
