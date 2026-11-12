//! The application's routes to a running Load Control's configuration
//! (#2497): Shed_Levels, Full_Duty_Baseline and State_Description change
//! while the server holds the object, a request under way is looked at again,
//! and what that moves is reported to a SubscribeCOV subscriber and saved
//! with the request.

use super::*;
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_types::enums::ShedState;
use bacnet_types::error::RemoteError;

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
            if c == class && e == code),
        "expected {class:?}/{code:?}, got {error:?}"
    );
}

/// `property` of LC-1 (`index` of it), from the server's database.
async fn read_value(
    h: &Harness,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Option<PropertyValue> {
    h.server
        .database()
        .read()
        .await
        .get(&lc1())
        .unwrap()
        .read_property(property, index)
        .ok()
}

/// SubscribeCOVProperty on `property` of LC-1.
async fn subscribe_property(h: &mut Harness, property: PropertyIdentifier) {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 77,
        monitored_object_identifier: lc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: property,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
    assert_eq!(response(h).await, Ok(()));
}

/// A request for level 2, for 30 minutes from 15:05, that the application
/// has found compliant: LC-1 reads SHED_COMPLIANT, and its reports so far
/// have been taken.
async fn complying(h: &mut Harness, app: &Arc<App>) {
    set_clock(h, 0);
    assert_eq!(
        subscribed(h, lc1()).await,
        report(0, 0, unspecified(), 0, 0)
    );
    for (property, value) in [
        (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x02]),
        (PropertyIdentifier::SHED_DURATION, unsigned(30)),
        (PropertyIdentifier::DUTY_WINDOW, unsigned(10)),
        (PropertyIdentifier::START_TIME, start_at(5)),
    ] {
        write(h, lc1(), property, value).await;
        h.cov_notification().await;
    }
    // 15:05 comes, and the loads comply.
    app.answer(ShedProgress::Compliant, 2);
    set_clock(h, 5);
    assert_eq!(
        values(&h.cov_notification().await, lc1()),
        report(2, 2, start_at(5), 30, 10)
    );
}

#[tokio::test(start_paused = true)]
async fn new_shed_levels_reach_a_request_under_way_and_are_reported_and_saved() {
    let app = app();
    let store = Arc::new(Store::default());
    let mut h = kept_harness(&app, &store).await;
    complying(&mut h, &app).await;
    assert_eq!(
        app.asked.lock().unwrap().last().unwrap().shed_level,
        Some(1)
    );

    // The loads now fall short. The route puts the request to them again,
    // with no clock tick between, so the state is read straight after it.
    app.answer(ShedProgress::Shedding, 1);
    h.server
        .set_load_control_shed_levels_local(
            &lc1(),
            vec![
                (2, "Raise setpoints".into()),
                (4, "Stop the chillers".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(ShedState::SHED_NON_COMPLIANT.to_raw())
    );
    {
        let asked = app.asked.lock().unwrap();
        let request = asked.last().unwrap();
        assert_eq!(
            request.shed_level,
            Some(0),
            "level 2 is now the first entry"
        );
        assert!(request.started);
    }
    assert_eq!(
        values(&h.cov_notification().await, lc1()),
        report(3, 2, start_at(5), 30, 10)
    );
    // The array and its descriptions went together, and their number changed.
    assert_eq!(
        read_value(&h, PropertyIdentifier::SHED_LEVELS, Some(0)).await,
        Some(PropertyValue::Unsigned(2))
    );
    assert_eq!(
        read_value(&h, PropertyIdentifier::SHED_LEVELS, Some(2)).await,
        Some(PropertyValue::Unsigned(4))
    );
    assert_eq!(
        read_value(&h, PropertyIdentifier::SHED_LEVEL_DESCRIPTIONS, Some(1)).await,
        Some(PropertyValue::CharacterString("Raise setpoints".into()))
    );

    // The state change was saved, with nobody waiting; the levels aren't
    // part of what storage keeps. Stopping waits for the save.
    h.server.stop().await.unwrap();
    drop(h);
    let saved = store.saved.lock().unwrap().clone().unwrap();
    assert_eq!(saved.state, ShedState::SHED_NON_COMPLIANT);
    assert_eq!(saved.requested_shed_level, BACnetShedLevel::Level(2));
}

#[tokio::test(start_paused = true)]
async fn shed_levels_refuse_what_a_write_would_and_name_their_errors() {
    let app = app();
    let mut h = harness(&app).await;
    // A level of 0 is the no-shed state, which the array doesn't hold.
    let error = h
        .server
        .set_load_control_shed_levels_local(&lc1(), vec![(1, "a".into()), (0, "b".into())])
        .await
        .unwrap_err();
    assert_error(error, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(
        read_value(&h, PropertyIdentifier::SHED_LEVELS, Some(0)).await,
        Some(PropertyValue::Unsigned(3)),
        "nothing changed"
    );
    // An unknown object, and one that isn't a Load Control.
    let unknown = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 9).unwrap();
    for error in [
        h.server
            .set_load_control_shed_levels_local(&unknown, vec![])
            .await,
        h.server
            .set_load_control_full_duty_baseline_local(&unknown, None)
            .await,
        h.server
            .set_load_control_state_description_local(&unknown, None)
            .await,
    ] {
        assert_error(
            error.unwrap_err(),
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        );
    }
    for error in [
        h.server
            .set_load_control_shed_levels_local(&av1(), vec![])
            .await,
        h.server
            .set_load_control_full_duty_baseline_local(&av1(), Some(1.0))
            .await,
        h.server
            .set_load_control_state_description_local(&av1(), None)
            .await,
    ] {
        assert_error(
            error.unwrap_err(),
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        );
    }
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_new_baseline_reaches_the_shedder_and_what_it_moves_is_reported_and_saved() {
    let app = app();
    let store = Arc::new(Store::default());
    let mut h = kept_harness(&app, &store).await;
    complying(&mut h, &app).await;
    assert_eq!(
        app.asked.lock().unwrap().last().unwrap().full_duty_baseline,
        None
    );
    assert_eq!(
        read_value(&h, PropertyIdentifier::FULL_DUTY_BASELINE, None).await,
        None,
        "served only once set"
    );

    app.answer(ShedProgress::Shedding, 1);
    h.server
        .set_load_control_full_duty_baseline_local(&lc1(), Some(250.0))
        .await
        .unwrap();
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(ShedState::SHED_NON_COMPLIANT.to_raw())
    );
    assert_eq!(
        app.asked.lock().unwrap().last().unwrap().full_duty_baseline,
        Some(250.0)
    );
    assert_eq!(
        read_value(&h, PropertyIdentifier::FULL_DUTY_BASELINE, None).await,
        Some(PropertyValue::Real(250.0))
    );
    assert_eq!(
        values(&h.cov_notification().await, lc1()),
        report(3, 2, start_at(5), 30, 10)
    );

    // A negative or non-finite baseline is refused and changes nothing.
    for bad in [-1.0, f32::NAN, f32::INFINITY] {
        let error = h
            .server
            .set_load_control_full_duty_baseline_local(&lc1(), Some(bad))
            .await
            .unwrap_err();
        assert_error(error, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    }
    assert_eq!(
        read_value(&h, PropertyIdentifier::FULL_DUTY_BASELINE, None).await,
        Some(PropertyValue::Real(250.0))
    );

    // Clearing it takes the property away again, and tells the shedder.
    app.answer(ShedProgress::Compliant, 2);
    h.server
        .set_load_control_full_duty_baseline_local(&lc1(), None)
        .await
        .unwrap();
    assert!(
        read_value(&h, PropertyIdentifier::FULL_DUTY_BASELINE, None)
            .await
            .is_none()
    );
    assert_eq!(
        app.asked.lock().unwrap().last().unwrap().full_duty_baseline,
        None
    );
    assert_eq!(
        values(&h.cov_notification().await, lc1()),
        report(2, 2, start_at(5), 30, 10)
    );
    h.server.stop().await.unwrap();
    drop(h);
    let saved = store.saved.lock().unwrap().clone().unwrap();
    assert_eq!(saved.state, ShedState::SHED_COMPLIANT);
}

#[tokio::test(start_paused = true)]
async fn a_state_description_follows_the_state_and_reaches_property_subscribers() {
    let app = app();
    let mut h = harness(&app).await;
    assert!(
        read_value(&h, PropertyIdentifier::STATE_DESCRIPTION, None)
            .await
            .is_none(),
        "served only once set"
    );
    complying(&mut h, &app).await;
    h.server
        .set_load_control_state_description_local(&lc1(), Some("Idle".into()))
        .await
        .unwrap();
    subscribe_property(&mut h, PropertyIdentifier::STATE_DESCRIPTION).await;
    let text = |text: &str| encode(PropertyValue::CharacterString(text.into()));
    let described = |note: &str| {
        vec![
            (PropertyIdentifier::STATE_DESCRIPTION, text(note)),
            (SF, normal()),
        ]
    };
    assert_eq!(
        values(&h.cov_notification().await, lc1()),
        described("Idle")
    );

    // A change is reported, and the request is not looked at again: the
    // loads' new answer waits for the next pass.
    app.answer(ShedProgress::Shedding, 1);
    let asked = app.asked.lock().unwrap().len();
    h.server
        .set_load_control_state_description_local(&lc1(), Some("Shedding 2 of 3".into()))
        .await
        .unwrap();
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(ShedState::SHED_COMPLIANT.to_raw())
    );
    assert_eq!(app.asked.lock().unwrap().len(), asked);
    assert_eq!(
        values(&h.cov_notification().await, lc1()),
        described("Shedding 2 of 3")
    );
    // The same text again reports nothing.
    h.server
        .set_load_control_state_description_local(&lc1(), Some("Shedding 2 of 3".into()))
        .await
        .unwrap();
    h.no_notification().await;
    // Cleared, the property goes.
    h.server
        .set_load_control_state_description_local(&lc1(), None)
        .await
        .unwrap();
    assert!(
        read_value(&h, PropertyIdentifier::STATE_DESCRIPTION, None)
            .await
            .is_none()
    );
    // The property is gone, so its subscriber hears nothing of that.
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_change_that_moves_nothing_sends_no_report() {
    let app = app();
    let mut h = harness(&app).await;
    complying(&mut h, &app).await;
    // The loads answer as before, so neither a new array nor a new
    // baseline moves anything the SubscribeCOV row carries.
    let asked = app.asked.lock().unwrap().len();
    h.server
        .set_load_control_shed_levels_local(&lc1(), vec![(1, "a".into()), (2, "b".into())])
        .await
        .unwrap();
    h.server
        .set_load_control_full_duty_baseline_local(&lc1(), Some(100.0))
        .await
        .unwrap();
    assert_eq!(app.asked.lock().unwrap().len(), asked + 2, "both looked");
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_restored_request_is_put_to_the_shedder_with_the_levels_given_at_build() {
    let app = app();
    app.answer(ShedProgress::Compliant, 2);
    let (_, _, store, _) = restored(&app);
    // As the Python keywords and the object's setters give them, before the
    // object joins the server: the first look at the restored request has
    // its level.
    let mut h = kept_harness(&app, &store).await;
    {
        let asked = app.asked.lock().unwrap();
        assert_eq!(asked.first().unwrap().shed_level, Some(1));
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn written_levels_win_at_a_restart_until_the_route_replaces_them() {
    let app = app();
    app.answer(ShedProgress::Compliant, 2);
    let (_, _, store, _) = restored(&app);
    let mut h = kept_harness(&app, &store).await;
    assert_eq!(
        app.asked.lock().unwrap().last().unwrap().shed_level,
        Some(1)
    );
    // A client rewrites the array whole, over the wire: the requested
    // level 2 is its first element now, and the request is looked at again.
    let mut written = unsigned(2);
    written.extend(unsigned(5));
    write(&mut h, lc1(), PropertyIdentifier::SHED_LEVELS, written).await;
    assert_eq!(
        app.asked.lock().unwrap().last().unwrap().shed_level,
        Some(0)
    );
    assert_eq!(
        store.saved.lock().unwrap().as_ref().unwrap().shed_levels,
        [(1, 2), (2, 5)]
    );
    h.server.stop().await.unwrap();
    drop(h);

    // Back with the same levels given at build: the written ones win, and
    // the restored request is put to the shedder with them.
    app.asked.lock().unwrap().clear();
    let mut h = kept_harness(&app, &store).await;
    assert_eq!(
        app.asked.lock().unwrap().first().unwrap().shed_level,
        Some(0)
    );
    assert_eq!(
        read_value(&h, PropertyIdentifier::SHED_LEVELS, Some(1)).await,
        Some(PropertyValue::Unsigned(2))
    );
    // The application's route replaces them, and storage forgets them.
    h.server
        .set_load_control_shed_levels_local(&lc1(), vec![(1, "a".into()), (2, "b".into())])
        .await
        .unwrap();
    h.server.stop().await.unwrap();
    drop(h);
    assert!(
        store
            .saved
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .shed_levels
            .is_empty()
    );
    app.asked.lock().unwrap().clear();
    let mut h = kept_harness(&app, &store).await;
    assert_eq!(
        app.asked.lock().unwrap().first().unwrap().shed_level,
        Some(1)
    );
    h.server.stop().await.unwrap();
}
