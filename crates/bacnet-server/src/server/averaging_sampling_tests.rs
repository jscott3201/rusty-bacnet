//! The running server samples an Averaging object's Object_Property_Reference
//! itself (Clauses 12.5.14 and 12.5.15, #1144), on the monotonic operation
//! task, every Window_Interval / Window_Samples seconds.
//!
//! Time is paused and moved with `tokio::time::advance`; yielding, rather than
//! sleeping, lets the task run without moving the clock further.
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::EngineeringUnits;
use tokio::time::Instant;

const ATTEMPTED: PropertyIdentifier = PropertyIdentifier::ATTEMPTED_SAMPLES;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

fn avg2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AVERAGING, 2).unwrap()
}

/// A server whose AVG-1 averages `target`'s Present_Value over 10 s in 5
/// samples (one every 2 s), next to AVG-2, which has no reference. The
/// schedule starts when the server binds its clock, at the returned instant.
async fn start_sampling(target: ObjectIdentifier) -> (Harness, Instant) {
    let origin = Instant::now();
    let h = Harness::start_with(ServerConfig::default(), |db| {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.set_window_interval(10).unwrap();
        avg.set_window_samples(5).unwrap();
        avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
            target,
            PV.to_raw(),
        )));
        db.add(Box::new(avg)).unwrap();
        db.add(Box::new(AveragingObject::new(2, "AVG-2").unwrap()))
            .unwrap();
    })
    .await;
    // Startup takes no paused time, so the schedule's origin is `origin`.
    assert_eq!(Instant::now(), origin);
    (h, origin)
}

/// Let the server run everything that is ready without moving the clock.
async fn run_ready() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

/// Move the paused clock to `millis` after `origin` and let the server run.
async fn advance_to(origin: Instant, millis: u64) {
    let target = origin + Duration::from_millis(millis);
    tokio::time::advance(target.saturating_duration_since(Instant::now())).await;
    run_ready().await;
}

async fn read(h: &Harness, oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// `(Attempted_Samples, Valid_Samples)` of AVG-1.
async fn counts(h: &Harness) -> (u64, u64) {
    let unsigned = |value| match value {
        PropertyValue::Unsigned(count) => count,
        other => panic!("a count is Unsigned: {other:?}"),
    };
    (
        unsigned(read(h, avg1(), ATTEMPTED).await),
        unsigned(read(h, avg1(), VALID).await),
    )
}

#[tokio::test(start_paused = true)]
async fn averaging_server_samples_its_reference_every_window_spacing() {
    let (mut h, origin) = start_sampling(av1()).await;
    advance_to(origin, 1_999).await;
    assert_eq!(counts(&h).await, (0, 0), "nothing before the first spacing");
    advance_to(origin, 2_000).await;
    assert_eq!(counts(&h).await, (1, 1));
    assert_eq!(avg_value(&h, AVG).await, PropertyValue::Real(0.0));

    h.write_local(10.0).await;
    advance_to(origin, 3_999).await;
    assert_eq!(counts(&h).await, (1, 1));
    advance_to(origin, 4_000).await;
    assert_eq!(counts(&h).await, (2, 2));
    assert_eq!(avg_value(&h, AVG).await, PropertyValue::Real(5.0));
    assert_eq!(avg_value(&h, MAX).await, PropertyValue::Real(10.0));

    // The window is full once Window_Interval has passed; later samples push
    // the oldest out.
    for millis in [6_000, 8_000, 10_000] {
        advance_to(origin, millis).await;
    }
    assert_eq!(counts(&h).await, (5, 5));
    assert_eq!(avg_value(&h, MIN).await, PropertyValue::Real(0.0));
    advance_to(origin, 12_000).await;
    assert_eq!(counts(&h).await, (5, 5));
    assert_eq!(avg_value(&h, MIN).await, PropertyValue::Real(10.0));

    // An Averaging object without a reference is left to the application.
    assert_eq!(
        read(&h, avg2(), ATTEMPTED).await,
        PropertyValue::Unsigned(0)
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_server_counts_a_missing_reference_as_a_missed_attempt() {
    let av9 = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap();
    let (mut h, origin) = start_sampling(av9).await;
    advance_to(origin, 2_000).await;
    assert_eq!(counts(&h).await, (1, 0));
    assert!(matches!(avg_value(&h, AVG).await, PropertyValue::Real(v) if v.is_nan()));

    // Once the referenced object exists, the next sample reads it.
    h.server
        .database()
        .write()
        .await
        .add(Box::new(
            AnalogValueObject::new(9, "AV-9", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    advance_to(origin, 4_000).await;
    assert_eq!(counts(&h).await, (2, 1));
    assert_eq!(avg_value(&h, AVG).await, PropertyValue::Real(0.0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_window_write_restarts_the_server_schedule() {
    let (mut h, origin) = start_sampling(av1()).await;
    advance_to(origin, 1_500).await;
    // Writing the value it already has still empties the window and starts
    // the schedule over from the write.
    h.server
        .write_local(
            &avg1(),
            PropertyIdentifier::WINDOW_SAMPLES,
            None,
            PropertyValue::Unsigned(5),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    advance_to(origin, 2_000).await;
    assert_eq!(counts(&h).await, (0, 0), "the old due time has passed");
    advance_to(origin, 3_499).await;
    assert_eq!(counts(&h).await, (0, 0));
    advance_to(origin, 3_500).await;
    assert_eq!(counts(&h).await, (1, 1));

    // A new Window_Interval sets a new spacing from the write: 20 s over 5
    // samples, 4 s apart.
    h.server
        .write_local(
            &avg1(),
            PropertyIdentifier::WINDOW_INTERVAL,
            None,
            PropertyValue::Unsigned(20),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(counts(&h).await, (0, 0));
    advance_to(origin, 7_499).await;
    assert_eq!(counts(&h).await, (0, 0));
    advance_to(origin, 7_500).await;
    assert_eq!(counts(&h).await, (1, 1));
    advance_to(origin, 11_500).await;
    assert_eq!(counts(&h).await, (2, 2));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_server_samples_notify_property_subscribers() {
    let (mut h, _) = start_sampling(av1()).await;
    subscribe_property(&mut h, 1, AVG, None).await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(values(&h.cov_notification().await), [(AVG, real(f32::NAN))]);
    subscribe_property(&mut h, 2, ATTEMPTED, None).await;
    assert_eq!(response(&h).await, Ok(()));
    h.cov_notification().await;

    // Waiting for a notification lets paused time run to the next sample.
    assert_eq!(
        notifications(&h, 2).await,
        BTreeMap::from([
            (1, vec![(AVG, real(0.0))]),
            (2, vec![(ATTEMPTED, encoded(PropertyValue::Unsigned(1)))]),
        ])
    );
    h.write_local(10.0).await;
    assert_eq!(
        notifications(&h, 2).await,
        BTreeMap::from([
            (1, vec![(AVG, real(5.0))]),
            (2, vec![(ATTEMPTED, encoded(PropertyValue::Unsigned(2)))]),
        ])
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_application_samples_join_a_server_sampled_window() {
    let (mut h, origin) = start_sampling(av1()).await;
    advance_to(origin, 1_000).await;
    sample(&h, 4.0).await;
    h.server
        .add_averaging_sample_local(&avg1(), None)
        .await
        .unwrap();
    assert_eq!(counts(&h).await, (2, 1));
    // The server's own sample still comes at 2 s.
    advance_to(origin, 1_999).await;
    assert_eq!(counts(&h).await, (2, 1));
    advance_to(origin, 2_000).await;
    assert_eq!(counts(&h).await, (3, 2));
    assert_eq!(avg_value(&h, AVG).await, PropertyValue::Real(2.0));
    h.server.stop().await.unwrap();
}
