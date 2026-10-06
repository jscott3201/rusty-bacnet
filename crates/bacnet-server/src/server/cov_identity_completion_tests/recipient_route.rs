use super::*;
use crate::cov::{CovObservation, CovSample};

fn routed(single: bool, confirmed: bool) -> CovSubscription {
    let mut sub = proposal(
        CovNotificationKind::Single,
        confirmed,
        PropertyIdentifier::PRESENT_VALUE,
    );
    sub.monitored_property = single.then_some(PropertyIdentifier::PRESENT_VALUE);
    sub.expires_at = Some(runtime_clock::now() + Duration::from_secs(600));
    sub.subscriber_network = Some(bacnet_encoding::npdu::NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[4]),
    });
    sub
}
fn observation(value: f32) -> CovObservation {
    CovObservation::new(CovSample::new(&PropertyValue::Real(value)).unwrap(), None).unwrap()
}
async fn migrate(fixture: &Fixture, old: &CovSubscriptionSnapshot) -> CovSubscriptionSnapshot {
    let mut sub = (**old).clone();
    sub.subscriber_mac = MacAddr::from_slice(&[127, 0, 0, 9, 0xba, 0xd0]);
    // Same reset as an admitted ordinary/Single handler renewal, unlike a
    // Multiple context refresh retaining unincluded-reference observations.
    sub.last_notified_observation = None;
    let mut table = fixture.table.write().await;
    let current = table.subscribe(sub).unwrap();
    assert_eq!(old.key(), current.key());
    assert!(!table.is_current(old));
    current
}

#[tokio::test]
async fn cov_recipient_route_held_unconfirmed_success_cannot_overwrite_renewal() {
    for single in [false, true] {
        for initial in [false, true] {
            let fixture = Fixture::new(true);
            let old = fixture
                .table
                .write()
                .await
                .subscribe(routed(single, false))
                .unwrap();
            let snapshots = [old.clone()];
            let mut work = Box::pin(fixture.fire(initial, &snapshots));
            assert!(futures_util::poll!(work.as_mut()).is_pending());
            assert_eq!(
                fixture.routes.lock().unwrap().as_slice(),
                std::slice::from_ref(&old.subscriber_mac)
            );
            let current = migrate(&fixture, &old).await;
            fixture.release.add_permits(1);
            tokio::time::timeout(Duration::from_secs(2), work)
                .await
                .unwrap();
            assert_eq!(
                fixture
                    .table
                    .read()
                    .await
                    .get_subscription(old.key())
                    .unwrap()
                    .last_notified_observation,
                None
            );
            fixture.release.add_permits(1);
            fixture.fire(initial, std::slice::from_ref(&current)).await;
            assert_eq!(
                fixture.routes.lock().unwrap().as_slice(),
                &[old.subscriber_mac.clone(), current.subscriber_mac.clone()]
            );
            assert_eq!(
                decode_npdu(fixture.sent.lock().unwrap()[1].clone())
                    .unwrap()
                    .destination,
                old.subscriber_network
            );
            assert_eq!(
                fixture
                    .table
                    .read()
                    .await
                    .get_subscription(current.key())
                    .unwrap()
                    .last_notified_observation
                    .as_ref()
                    .unwrap()
                    .sample(),
                &CovSample::new(&PropertyValue::Real(10.0)).unwrap()
            );
            fixture.finish(false).await;
        }
    }
}

#[tokio::test]
async fn cov_recipient_route_renewal_before_admission_fences_both_initial_forms() {
    for single in [false, true] {
        for confirmed in [false, true] {
            let fixture = Fixture::new(false);
            let old = fixture
                .table
                .write()
                .await
                .subscribe(routed(single, confirmed))
                .unwrap();
            let snapshots = [old.clone()];
            let guard = fixture.db.write().await;
            let mut work = Box::pin(fixture.fire(true, &snapshots));
            assert!(futures_util::poll!(work.as_mut()).is_pending());
            migrate(&fixture, &old).await;
            drop(guard);
            tokio::time::timeout(Duration::from_secs(2), work)
                .await
                .unwrap();
            assert!(fixture.sent.lock().unwrap().is_empty());
            assert_eq!(fixture.transactions.active_count(), 0);
            assert_eq!(
                fixture
                    .table
                    .read()
                    .await
                    .get_subscription(old.key())
                    .unwrap()
                    .last_notified_observation,
                None
            );
            fixture.finish(false).await;
        }
    }
}

#[tokio::test]
async fn cov_recipient_route_admitted_confirmed_ack_cannot_overwrite_new_generation() {
    for single in [false, true] {
        let fixture = Fixture::new(true);
        let old = fixture
            .table
            .write()
            .await
            .subscribe(routed(single, true))
            .unwrap();
        fixture.fire(true, std::slice::from_ref(&old)).await;
        tokio::time::timeout(Duration::from_secs(2), fixture.entered.notified())
            .await
            .unwrap();
        assert_eq!(
            fixture
                .table
                .read()
                .await
                .get_subscription(old.key())
                .unwrap()
                .last_notified_observation
                .as_ref()
                .unwrap()
                .sample(),
            &CovSample::new(&PropertyValue::Real(1.0)).unwrap(),
            "confirmed baseline waits for the ACK (#896)"
        );
        let current = migrate(&fixture, &old).await;
        assert_eq!(
            current.last_notified_observation, None,
            "ordinary/Single renewal resets its observation"
        );
        assert!(fixture
            .table
            .write()
            .await
            .complete_for_test(&current, observation(99.0)));
        assert_eq!(
            fixture.routes.lock().unwrap().as_slice(),
            std::slice::from_ref(&old.subscriber_mac)
        );
        let npdu = decode_npdu(fixture.sent.lock().unwrap()[0].clone()).unwrap();
        assert_eq!(npdu.destination, old.subscriber_network);
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("confirmed COV");
        };
        fixture.release.add_permits(1);
        assert!(fixture.transactions.admit_terminal(
            &old.subscriber_mac,
            old.subscriber_network.as_ref(),
            None,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice
            })
        ));
        // Let the worker handle the ACK before shutdown can cancel it.
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), fixture.transactions.join_next())
                .await
                .unwrap(),
            Some(Ok(()))
        ));
        fixture.finish(false).await;
        assert_eq!(
            fixture
                .table
                .read()
                .await
                .get_subscription(current.key())
                .unwrap()
                .last_notified_observation,
            Some(observation(99.0))
        );
    }
}
