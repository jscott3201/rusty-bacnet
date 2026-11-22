//! The session's stop and a full client's task that no abort can end: one
//! inside the link's send, which runs synchronously on its worker thread.
//!
//! Real time, by design: the send is held on a worker of a multi-threaded
//! runtime, which paused time can't stand in for. The one wait is a short
//! release of that send (300 ms), and the test takes about that long.
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// When the test lets the held send go, whatever the stop does.
const RELEASE: Duration = Duration::from_millis(300);

/// A Confirmed-COV-Notification from node [`OTHER`] under `invoke_id`,
/// which the full client acknowledges.
fn notification(invoke_id: u8) -> Apdu {
    let mut parameters = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 4,
        initiating_device_identifier: oid(ObjectType::DEVICE, 3003),
        monitored_object_identifier: ai1(),
        time_remaining: 60,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x41, 0xa0, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut parameters)
    .unwrap();
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        &parameters,
    )
}

/// A full client's task inside the link's send, held there on its worker
/// so no abort can end it, has returned before the session's stop reaches
/// the link's stop (#2617): the stop waits for the client's hold on the
/// seat. (An await the abort can end is not waited for: see
/// `the_sessions_stop_does_not_wait_on_a_client_task_stuck_in_a_turn`.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sessions_stop_reaches_the_link_only_once_a_client_send_returns() {
    let wire = Wire::new();
    let hold = Arc::new(AtomicBool::new(false));
    let in_send = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicBool::new(false));
    let stopped_in_send = Arc::new(AtomicBool::new(false));
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let port = wire
        .port(NODE)
        .on_send({
            let (hold, in_send, entered, release_rx) = (
                Arc::clone(&hold),
                Arc::clone(&in_send),
                Arc::clone(&entered),
                Arc::clone(&release_rx),
            );
            // The client's acknowledgment to OTHER waits here, on the
            // worker, until the test or the link's stop lets it go.
            move |sent| {
                if sent.mac.as_slice() == Wire::mac(OTHER).as_slice()
                    && hold.swap(false, Ordering::SeqCst)
                {
                    in_send.store(true, Ordering::SeqCst);
                    entered.store(true, Ordering::SeqCst);
                    let released = release_rx.lock().unwrap();
                    let _ = released.recv_timeout(WAIT);
                    in_send.store(false, Ordering::SeqCst);
                }
                async { Ok(()) }
            }
        })
        .before_stop({
            let (in_send, stopped_in_send, release_tx) = (
                Arc::clone(&in_send),
                Arc::clone(&stopped_in_send),
                release_tx.clone(),
            );
            move || {
                stopped_in_send.store(in_send.load(Ordering::SeqCst), Ordering::SeqCst);
                let _ = release_tx.send(());
                async { Ok(()) }
            }
        });
    let node_link = port.handle();
    let identity = DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(20.0);
    let database = build_database_with_extra(&identity, vec![Box::new(input)]).unwrap();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(identity)
            .with_database(database);
    session.start().await.unwrap();
    let client = session.client().unwrap();
    let _notifications = client.cov_notifications();

    hold.store(true, Ordering::SeqCst);
    wire.deliver(
        &Wire::mac(NODE),
        wire::frame(&Wire::mac(OTHER), npdu_of(&notification(7), true), false),
    );
    eventually(
        WAIT,
        "the client's acknowledgment in the link's send",
        async || entered.load(Ordering::SeqCst),
    )
    .await;

    // A stop that waits reaches the link only after this lets the send go.
    let release = std::thread::spawn(move || {
        std::thread::sleep(RELEASE);
        let _ = release_tx.send(());
    });
    tokio::time::timeout(WAIT, session.stop())
        .await
        .expect("the stop returns")
        .expect("the stop succeeds");
    release.join().unwrap();
    assert_eq!(node_link.stops(), 1);
    assert!(
        !stopped_in_send.load(Ordering::SeqCst),
        "the link stopped while the client's send still ran"
    );
    drop(client);
}
