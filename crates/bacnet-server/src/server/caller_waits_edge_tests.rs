//! The parked and dropped calls of [`super`] on a multi-thread runtime, a
//! call whose task its runtime drops, and the calls' futures staying
//! `Send` and `Sync` (#2772).

use bacnet_objects::access_control::{
    AccessEventReport, AuthorizationRequest, CredentialReadReport, DoorStateReport,
};
use bacnet_types::enums::{Polarity, Reliability};

use super::*;

/// A hang guard only: no test waits on it unless it has already failed.
const HANG: Duration = Duration::from_secs(10);

/// A write polled once while the application holds the database, then
/// parked, holds up no writer queued behind it once the guard is free,
/// with the server's tasks on threads of their own: the writer queues after
/// the parked call took its place, so the order is fixed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_parked_write_local_holds_up_no_writer_on_threads() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.write_local(
        &av,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(5.0),
        Some(8),
        crate::LocalCommandSource::ServerDevice,
    ));
    assert!(poll_once(&mut parked).is_pending());
    let writer = tokio::spawn({
        let db = Arc::clone(&node.server.db);
        async move { drop(db.write().await) }
    });
    drop(held);
    tokio::time::timeout(HANG, writer)
        .await
        .expect("the writer queued behind the parked call never got the database")
        .unwrap();
    parked.await.unwrap();
    assert_eq!(
        node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(5.0)
    );
    node.server.stop().await.unwrap();
}

/// A write whose caller goes while its task waits makes no change, and its
/// task ends without the guard, with the server's tasks on threads of
/// their own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_dropped_while_its_task_waits_changes_nothing_on_threads() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let before = node.value(av, PropertyIdentifier::PRESENT_VALUE).await;
    // The read may have waited in a task of its own, still in the set.
    node.settled().await;
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.write_local(
        &av,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(5.0),
        Some(8),
        crate::LocalCommandSource::ServerDevice,
    ));
    assert!(poll_once(&mut parked).is_pending());
    assert_eq!(node.server.request_tasks.len(), 1, "one task waits");
    drop(parked);
    let tasks = &node.server.request_tasks;
    tokio::time::timeout(HANG, async {
        while {
            tasks.reap_finished();
            !tasks.is_empty()
        } {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the task of a dropped call never ended");
    drop(held);
    assert_eq!(
        node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
        before
    );
    node.server.stop().await.unwrap();
}

/// A read whose task the server's set drops before it reads, as a runtime
/// that shuts down would, reads in the caller instead.
#[tokio::test(start_paused = true)]
async fn generate_pics_reads_in_the_caller_when_its_task_is_dropped() {
    let mut node = node().await;
    let config = crate::pics::PicsConfig::default();
    let expected = format!("{:?}", node.server.generate_pics(&config).await);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.generate_pics(&config));
    assert!(poll_once(&mut parked).is_pending());
    node.server.request_tasks.close();
    drop(held);
    assert_eq!(format!("{:?}", parked.await), expected);
    node.server.stop().await.unwrap();
}

/// A write whose task is dropped before it takes the guard makes no
/// change, and says the server has gone.
#[tokio::test(start_paused = true)]
async fn a_write_whose_task_is_dropped_changes_nothing() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let before = node.value(av, PropertyIdentifier::PRESENT_VALUE).await;
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.write_local(
        &av,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(5.0),
        Some(8),
        crate::LocalCommandSource::ServerDevice,
    ));
    assert!(poll_once(&mut parked).is_pending());
    node.server.request_tasks.close();
    drop(held);
    assert!(parked.await.is_err());
    assert_eq!(
        node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
        before
    );
    node.server.stop().await.unwrap();
}

/// Every call that waits through the server's task set is still `Send` and
/// `Sync`, as it was before #2772, though its future now holds a boxed task
/// across an await: checked when this compiles.
#[allow(dead_code, clippy::too_many_arguments)]
fn the_local_calls_are_send_and_sync<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    oid: &ObjectIdentifier,
    request: AuthorizationRequest<'_>,
    pics: &crate::pics::PicsConfig,
    octets: &[u8],
    access: AccessEventReport,
    credential: CredentialReadReport,
    door: DoorStateReport,
) {
    fn check(_: impl Send + Sync) {}
    let (property, value) = (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Null);
    let source = crate::LocalCommandSource::ServerDevice;
    check(server.read_local(oid, property, None));
    check(server.write_local(oid, property, None, value.clone(), None, source));
    check(server.write_local_encoded(oid, property, None, octets, None, source));
    check(server.set_present_value_local(oid, value.clone()));
    check(server.set_controlled_variable_value_local(oid, value.clone()));
    check(server.add_averaging_sample_local(oid, None));
    check(server.set_tracking_value_local(oid, value.clone()));
    check(server.add_pulses_local(oid, 1));
    check(server.set_reliability_local(oid, Reliability::NO_FAULT_DETECTED));
    check(server.set_program_state_local(oid, value));
    check(server.set_physical_input_local(oid, true));
    check(server.set_polarity_local(oid, Polarity::NORMAL));
    check(server.physical_output_local(oid));
    check(server.report_access_event_local(oid, access));
    check(server.report_credential_read_local(oid, credential));
    check(server.report_door_state_local(oid, door));
    check(server.set_load_control_shed_levels_local(oid, Vec::new()));
    check(server.set_load_control_full_duty_baseline_local(oid, None));
    check(server.set_load_control_state_description_local(oid, None));
    check(server.set_life_safety_operation_expected_local(oid, LifeSafetyOperation::NONE));
    check(server.authorize_access_local(request, 0));
    check(server.set_clock_zone(ClockConfig::new(0, false).unwrap()));
    check(server.purge_audit_log(oid));
    check(server.report_device_reset_local());
    check(server.generate_pics(pics));
    check(server.remove_peer_subscriptions(octets, None));
}
