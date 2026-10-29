//! The ReinitializeDevice after-reply hook on a full server (#1565): it runs
//! once the SimpleACK has been handed to the link, and never for a request
//! answered with an Error or a reply that could not be handed over.
use super::*;
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_types::enums::ReinitializedState;

/// What the link held each time an after-reply hook ran: the replies sent
/// to the requester so far.
type Observed = Arc<StdMutex<Vec<Vec<Apdu>>>>;

/// How a test handler answers once it has registered its hook.
type Answer = fn() -> Result<(), Error>;

const INVOKE_ID: u8 = 9;

fn request(state: ReinitializedState, password: Option<&str>) -> ConfirmedRequestPdu {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: password.map(str::to_owned),
    }
    .encode(&mut data)
    .unwrap();
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: INVOKE_ID,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        service_request: data.freeze(),
    }
}

/// The replies the link carried to the requester.
fn replies(sent: &StdMutex<Vec<Bytes>>) -> Vec<Apdu> {
    sent.lock()
        .unwrap()
        .iter()
        .map(|bytes| decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap())
        .collect()
}

/// Install a handler that registers an after-reply hook, then answers with
/// `outcome`. Each run of the hook records the replies on the link then.
fn install(f: &mut Fixture, outcome: Answer) -> Observed {
    let observed = Observed::default();
    let (seen, responses) = (Arc::clone(&observed), Arc::clone(&f.transport.responses));
    f.server.config_mut().on_reinitialize = Some(Arc::new(move |context, _| {
        let (seen, responses) = (Arc::clone(&seen), Arc::clone(&responses));
        context.after_reply(move || seen.lock().unwrap().push(replies(&responses)));
        outcome()
    }));
    observed
}

/// Send `request` from `SOURCE` with no reply channel, so the reply goes
/// out on the link itself.
async fn over_the_link(f: &Fixture, request: ConfirmedRequestPdu) {
    BACnetServer::handle_confirmed_request(
        &f.server.test_services(),
        &f.server.confirmed_request_tracker,
        &f.server.request_tasks.spawner(),
        SOURCE,
        None,
        request,
        None,
    )
    .await;
}

fn is_ack(reply: &Apdu) -> bool {
    matches!(reply, Apdu::SimpleAck(ack)
        if ack.invoke_id == INVOKE_ID
            && ack.service_choice == ConfirmedServiceChoice::REINITIALIZE_DEVICE)
}

#[tokio::test(start_paused = true)]
async fn the_after_reply_hook_runs_once_the_simple_ack_is_on_the_wire() {
    for state in [
        ReinitializedState::COLDSTART,
        ReinitializedState::WARMSTART,
        ReinitializedState::START_BACKUP,
        ReinitializedState::END_BACKUP,
        ReinitializedState::START_RESTORE,
        ReinitializedState::END_RESTORE,
        ReinitializedState::ABORT_RESTORE,
        ReinitializedState::ACTIVATE_CHANGES,
    ] {
        let mut f = plain_server(reporter()).await;
        let observed = install(&mut f, || Ok(()));
        over_the_link(&f, request(state, None)).await;
        let runs = observed.lock().unwrap().clone();
        assert_eq!(runs.len(), 1, "{state:?}: the hook runs once");
        assert!(
            matches!(runs[0].as_slice(), [reply] if is_ack(reply)),
            "{state:?}: the SimpleACK was on the wire before the hook ran: {runs:?}"
        );
        f.server.stop().await.unwrap();
    }
}

/// On an MS/TP reply channel, the handoff to the channel is the send.
#[tokio::test(start_paused = true)]
async fn the_after_reply_hook_follows_the_reply_channel_handoff() {
    let mut f = plain_server(reporter()).await;
    let (tx, rx) = oneshot::channel();
    let rx = Arc::new(StdMutex::new(rx));
    let handed_off = Arc::new(StdMutex::new(Vec::new()));
    let (seen, waiting) = (Arc::clone(&handed_off), Arc::clone(&rx));
    f.server.config_mut().on_reinitialize = Some(Arc::new(move |context, _| {
        let (seen, waiting) = (Arc::clone(&seen), Arc::clone(&waiting));
        context.after_reply(move || {
            let reply = waiting.lock().unwrap().try_recv().ok();
            seen.lock().unwrap().push(reply);
        });
        Ok(())
    }));
    BACnetServer::handle_confirmed_request(
        &f.server.test_services(),
        &f.server.confirmed_request_tracker,
        &f.server.request_tasks.spawner(),
        SOURCE,
        None,
        request(ReinitializedState::WARMSTART, None),
        Some(tx),
    )
    .await;
    let runs = handed_off.lock().unwrap().clone();
    assert_eq!(runs.len(), 1);
    let reply = runs[0].clone().expect("the reply was in the channel first");
    assert!(is_ack(
        &decode_apdu(decode_npdu(reply).unwrap().payload).unwrap()
    ));
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_after_reply_hook_never_runs_for_a_refusal() {
    let cases: [(&str, Answer, Option<&str>, ErrorCode); 5] = [
        (
            "handler refusal",
            || {
                Err(Error::protocol(
                    ErrorClass::DEVICE,
                    ErrorCode::CONFIGURATION_IN_PROGRESS,
                ))
            },
            None,
            ErrorCode::CONFIGURATION_IN_PROGRESS,
        ),
        (
            "handler error",
            || Err(Error::Encoding("disk full".into())),
            None,
            ErrorCode::OTHER,
        ),
        (
            "handler panic",
            || panic!("handler panic after registering"),
            None,
            ErrorCode::OTHER,
        ),
        (
            "missing password",
            || Ok(()),
            None,
            ErrorCode::PASSWORD_FAILURE,
        ),
        (
            "wrong password",
            || Ok(()),
            Some("wrong"),
            ErrorCode::PASSWORD_FAILURE,
        ),
    ];
    for (case, outcome, password, code) in cases {
        let mut f = plain_server(reporter()).await;
        let observed = install(&mut f, outcome);
        if code == ErrorCode::PASSWORD_FAILURE {
            f.server.config_mut().reinit_password = Some("required".into());
        }
        over_the_link(&f, request(ReinitializedState::WARMSTART, password)).await;
        let sent = replies(&f.transport.responses);
        assert!(
            matches!(sent.as_slice(), [Apdu::Error(error)] if error.error_code == code),
            "{case}: {sent:?}"
        );
        settle().await;
        assert!(observed.lock().unwrap().is_empty(), "{case}: hook ran");
        f.server.stop().await.unwrap();
    }
}

/// A SimpleACK the link refused was not handed over: the requester will
/// retry, so the restart waits for that.
#[tokio::test(start_paused = true)]
async fn the_after_reply_hook_never_runs_when_the_reply_is_not_handed_off() {
    let mut f = plain_server(reporter()).await;
    let observed = install(&mut f, || Ok(()));
    f.transport.fail_response.store(true, Ordering::Release);
    over_the_link(&f, request(ReinitializedState::COLDSTART, None)).await;
    assert!(matches!(
        replies(&f.transport.responses).as_slice(),
        [reply] if is_ack(reply)
    ));
    settle().await;
    assert!(observed.lock().unwrap().is_empty());
    f.server.stop().await.unwrap();
}
