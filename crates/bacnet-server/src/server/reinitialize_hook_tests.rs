//! After-reply hooks in isolation (#1565): registration order, a panicking
//! hook that leaves the next one running, and a clone the handler kept that
//! registers after the request was answered.
use std::sync::Mutex as StdMutex;

use super::*;
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;

fn request() -> ConfirmedRequestPdu {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 4,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        service_request: data.freeze(),
    }
}

async fn answer(handler: &ReinitializeHandler) -> (Apdu, AfterReply) {
    let db = RwLock::new(ObjectDatabase::new());
    let requester = Requester::new(&[1], None, TransportProvenance::unverified());
    response(
        &db,
        &request(),
        &None,
        Some(handler),
        requester,
        || Ok(()),
        |_, _| {},
    )
    .await
}

#[tokio::test]
async fn hooks_run_in_registration_order_past_a_panicking_one() {
    let ran = Arc::new(StdMutex::new(Vec::new()));
    let log = Arc::clone(&ran);
    let handler: ReinitializeHandler = Arc::new(move |context, _| {
        let first = Arc::clone(&log);
        context.after_reply(move || {
            first.lock().unwrap().push(1);
            panic!("first hook panics");
        });
        let second = Arc::clone(&log);
        context.after_reply(move || second.lock().unwrap().push(2));
        let third = Arc::clone(&log);
        context.after_reply(move || third.lock().unwrap().push(3));
        Ok(())
    });
    let (reply, hooks) = answer(&handler).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)));
    assert!(
        ran.lock().unwrap().is_empty(),
        "nothing before the hand-off"
    );
    hooks.run();
    assert_eq!(*ran.lock().unwrap(), [1, 2, 3]);
}

/// The hooks are taken under the database guard: a clone of the context
/// the handler kept registers too late, and that hook never runs.
#[tokio::test]
async fn a_hook_registered_through_a_kept_clone_never_runs() {
    let kept = Arc::new(StdMutex::new(None::<ReinitializeContext>));
    let ran = Arc::new(StdMutex::new(Vec::new()));
    let (slot, log) = (Arc::clone(&kept), Arc::clone(&ran));
    let handler: ReinitializeHandler = Arc::new(move |context, _| {
        let on_time = Arc::clone(&log);
        context.after_reply(move || on_time.lock().unwrap().push("on time"));
        *slot.lock().unwrap() = Some(context.clone());
        Ok(())
    });
    let (reply, hooks) = answer(&handler).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)));
    let late = Arc::clone(&ran);
    kept.lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .after_reply(move || late.lock().unwrap().push("late"));
    hooks.run();
    assert_eq!(*ran.lock().unwrap(), ["on time"]);
}
