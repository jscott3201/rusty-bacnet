//! A start refused once it has built its server (#2766): the start stops
//! the server and takes its handle back, so the database still comes back,
//! with what that server's start drove undone and what its first passes
//! did kept; and a refused start's wait for a handle still shared.
use super::*;
use bacnet_objects::load_control::{
    LoadControlObject, LoadControlPersistence, LoadControlSnapshot, LoadShedder, ShedRequest,
    ShedStatus,
};
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetObjectPropertyReference,
};
use bacnet_types::enums::ShedState;
use bacnet_types::primitives::{Date, Time};
use std::sync::Mutex as StdMutex;
use tokio::sync::Notify;

/// Storage holding a Load Control's request, which never fails.
struct Store(StdMutex<Option<LoadControlSnapshot>>);

impl LoadControlPersistence for Store {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

/// An application that sheds at once, counting the requests it is asked
/// to shed and its stops.
#[derive(Default)]
struct Shedder {
    asked: StdMutex<usize>,
    stops: StdMutex<usize>,
}

impl LoadShedder for Shedder {
    fn shed(&self, request: &ShedRequest) -> ShedStatus {
        *self.asked.lock().unwrap() += 1;
        let level = request.requested.clone();
        ShedStatus::new(
            bacnet_objects::load_control::ShedProgress::Compliant,
            level.clone(),
            level,
        )
    }

    fn stop(&self) {
        *self.stops.lock().unwrap() += 1;
    }
}

/// LC-1, restored with a shed under way for a century from 2020, which a
/// start offers `shedder`.
fn restored_load_control(shedder: &Arc<Shedder>) -> Box<dyn BACnetObject> {
    const CENTURY: u64 = 100 * 365 * 24 * 60;
    let mut saved = LoadControlSnapshot::default();
    saved.state = ShedState::SHED_COMPLIANT;
    saved.requested_shed_level = bacnet_types::constructed::BACnetShedLevel::Level(2);
    let new_year = Date {
        year: 120,
        month: 1,
        day: 1,
        day_of_week: 3,
    };
    let midnight = Time {
        hour: 0,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    saved.start_time = (new_year, midnight);
    saved.shed_duration = CENTURY;
    let store = Arc::new(Store(StdMutex::new(Some(saved))));
    let mut object = LoadControlObject::with_persistence(1, "LC-1", store).unwrap();
    object.set_shedder(Arc::clone(shedder) as Arc<dyn LoadShedder>);
    Box::new(object)
}

/// Start a session of both roles over `db` with a source Audit Reporter
/// the started link refuses, which it finds once its server is built: the
/// Audit recipient is another station before the link starts and the
/// node's own address after. Returns the ended session and its refusal.
async fn refused_once_built(
    mut db: ObjectDatabase,
) -> (EndpointSession<TestTransport>, StartError) {
    provision(&mut db, Wire::mac(NODE).as_slice());
    let reporter = bacnet_objects::audit::AuditReporterObject::new(1, "Reporter").unwrap();
    let reporter_oid = reporter.object_identifier();
    db.add(Box::new(reporter)).unwrap();
    let link = Wire::new()
        .port(NODE)
        .local_mac_until_started(Wire::mac(9).as_slice())
        .build();
    let mut session = EndpointSession::new(link, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(identity())
        .with_database(db)
        .with_source_audit_reporter(reporter_oid);
    let refused = session.start().await.unwrap_err();
    (session, refused)
}

/// The refused start stops the server it built, takes its handle on the
/// database back, and hands the database over with the outputs the
/// server's start drove released, so a restored shed's application is
/// told to stop. The corrected session, with no source Audit, serves
/// every object.
#[tokio::test(start_paused = true)]
async fn a_refusal_after_the_server_is_built_hands_the_database_back() {
    let shedder = Arc::new(Shedder::default());
    let mut db = database();
    db.add(restored_load_control(&shedder)).unwrap();
    let (session, refused) = refused_once_built(db).await;
    assert!(*shedder.asked.lock().unwrap() > 0, "the server's start ran");
    assert!(!session.can_start());
    let (error, db) = handed_back(refused);
    assert!(error.remote().is_some(), "{error}");
    assert_eq!(*shedder.stops.lock().unwrap(), 1, "the shed is let go");
    let expected = objects(&db);
    assert!(expected.contains(&ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap()));
    serves_every_object(db).await;
}

/// The server the start built ran before the refusal: its periodic work
/// had a first pass, whose results the database keeps. Here the Schedule
/// wrote its value to AV-7 at its priority, and the Trend Log logged AV-7.
#[tokio::test(start_paused = true)]
async fn a_refusal_after_the_server_is_built_keeps_its_first_passes() {
    let mut db = database();
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(21.5)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            av7(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    let mut trend = TrendLogObject::new(1, "TL-1", 8).unwrap();
    trend
        .set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: av7(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        }))
        .unwrap();
    trend
        .write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(6_000),
            None,
        )
        .unwrap();
    let trend_oid = trend.object_identifier();
    db.add(Box::new(schedule)).unwrap();
    db.add(Box::new(trend)).unwrap();
    let read = |db: &ObjectDatabase, oid, property| {
        db.get(&oid).unwrap().read_property(property, None).unwrap()
    };
    let (_, refused) = refused_once_built(db).await;
    let (_, db) = handed_back(refused);
    assert_eq!(
        read(&db, av7(), PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(21.5),
        "the Schedule's first pass commanded AV-7"
    );
    assert_eq!(
        read(&db, trend_oid, PropertyIdentifier::RECORD_COUNT),
        PropertyValue::Unsigned(1),
        "the Trend Log's first pass logged a record"
    );
    serves_every_object(db).await;
}

/// A session whose stop ran a server, with what a refused start's cleanup
/// runs, has its server's handle back once the server has stopped: the
/// session's is the only one.
#[tokio::test(start_paused = true)]
async fn a_stopped_server_holds_no_handle_on_the_database() {
    let wire = Wire::new();
    let (mut session, _) = default_session(&wire);
    session.start().await.unwrap();
    session.stop_server().await;
    assert_eq!(Arc::strong_count(session.database.as_ref().unwrap()), 1);
    session.stop().await.unwrap();
}

/// A stop of the server cancelled part way leaves the server with the
/// session, or its drop, to finish: the next stop completes, and the
/// session's handle is then the only one.
#[tokio::test(start_paused = true)]
async fn a_cancelled_server_stop_leaves_the_rest_to_the_next() {
    for polls in 1..4 {
        let wire = Wire::new();
        let (mut session, _) = default_session(&wire);
        session.start().await.unwrap();
        {
            let stopping = session.stop_server();
            tokio::pin!(stopping);
            for _ in 0..polls {
                if futures_util::poll!(stopping.as_mut()).is_ready() {
                    break;
                }
            }
        }
        session.stop().await.unwrap();
        let db = Arc::clone(session.database.as_ref().unwrap());
        for _ in 0..HAND_BACK_TURNS {
            if Arc::strong_count(&db) == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(Arc::strong_count(&db), 2, "after {polls} polls");
    }
}

/// A ClientOnly session refused once its link started (no request could
/// state its APDU size), whose link tells `stopped` as its stop ends.
fn refused_client(stopped: &Arc<Notify>) -> EndpointSession<TestTransport> {
    let stopped = Arc::clone(stopped);
    let link = Wire::new()
        .port(NODE)
        .after_stop(move || {
            stopped.notify_one();
            async { Ok(()) }
        })
        .build();
    let config = SessionConfig {
        max_apdu_length: 1,
        ..SessionConfig::default()
    };
    EndpointSession::new(link, SessionRole::ClientOnly, config)
        .unwrap()
        .with_database(database())
}

/// A refused start that ends its session waits a few runtime turns after
/// its cleanup for a handle on the database still held elsewhere: one let
/// go of within them leaves the database to come back. One held past them
/// leaves it with the ended session, the refusal carrying none, and
/// `can_start` tells that from a refusal the session can retry.
#[tokio::test(start_paused = true)]
async fn a_refused_start_waits_a_few_turns_for_a_shared_handle() {
    let stopped = Arc::new(Notify::new());
    let mut session = refused_client(&stopped);
    let held = Arc::clone(session.database.as_ref().unwrap());
    let holder = tokio::spawn({
        let stopped = Arc::clone(&stopped);
        async move {
            stopped.notified().await;
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            drop(held);
        }
    });
    let refused = session.start().await.unwrap_err();
    assert!(refused.database().is_some(), "{refused}");
    holder.await.unwrap();

    let mut session = refused_client(&stopped);
    let held = Arc::clone(session.database.as_ref().unwrap());
    let refused = session.start().await.unwrap_err();
    assert!(refused.database().is_none(), "{refused}");
    assert!(!session.can_start(), "the session ended");
    assert!(session.stop().await.is_err());
    drop(session);
    assert_eq!(Arc::strong_count(&held), 1, "the session let go of its own");
}
