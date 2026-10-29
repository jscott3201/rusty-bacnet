//! DeviceCommunicationControl through `ServerPolicy` (#1964): the policy is
//! asked after the password, static policy and source checks (and before the
//! disable rate charge), and a denial or a panic leaves the state and any
//! running timer as they were.
use super::*;
use crate::ServerPolicy;
use crate::device_communication_control::DeviceCommunicationControlAuthorizationContext;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Records each context it is asked about and answers `allow`.
#[derive(Default)]
struct Recording {
    allow: AtomicBool,
    seen: StdMutex<Vec<DeviceCommunicationControlAuthorizationContext>>,
}

impl ServerPolicy for Recording {
    fn authorize_device_communication_control(
        &self,
        context: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        self.seen.lock().unwrap().push(context.clone());
        self.allow.load(Ordering::Acquire)
    }
}

impl Recording {
    fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

fn config_with(policy: Arc<dyn ServerPolicy>) -> ServerConfig {
    ServerConfig {
        dcc_policy: DccPolicy::LegacyPermissive,
        policy: Some(policy),
        ..Default::default()
    }
}

fn timer() -> Arc<Mutex<crate::server::dcc_timer::TimerSlot>> {
    Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()))
}

#[tokio::test(start_paused = true)]
async fn a_policy_that_does_not_decide_dcc_allows_it() {
    struct Defaults;
    impl ServerPolicy for Defaults {}
    for policy in [
        Arc::new(Defaults) as Arc<dyn ServerPolicy>,
        Arc::new(crate::FnPolicy::new()),
    ] {
        let state = comm_state_in(DccState::Enable);
        let timer = timer();
        let response = dispatch_with_config(
            &state,
            &timer,
            EnableDisable::DISABLE_INITIATION,
            None,
            &config_with(policy),
        )
        .await;
        assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
        assert_eq!(state.get(), DccState::DisableInitiation);
    }
}

#[tokio::test(start_paused = true)]
async fn a_policy_denial_keeps_the_state_and_the_running_timer() {
    let policy = Arc::new(Recording::default());
    policy.allow.store(true, Ordering::Release);
    let config = config_with(policy.clone());
    let state = comm_state_in(DccState::Enable);
    let timer = timer();
    let response = dispatch_with_config(
        &state,
        &timer,
        EnableDisable::DISABLE_INITIATION,
        Some(5),
        &config,
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    assert_eq!(state.get(), DccState::DisableInitiation);
    assert!(timer.lock().await.is_some());

    policy.allow.store(false, Ordering::Release);
    assert_denied(dispatch_with_config(&state, &timer, EnableDisable::ENABLE, None, &config).await);
    assert_eq!(state.get(), DccState::DisableInitiation);
    assert!(timer.lock().await.is_some(), "a denial cancels no timer");

    // Each context carries the requested change; over B/IP there is no
    // principal, and Debug shows no address bytes.
    let seen = policy.seen.lock().unwrap().clone();
    let asked: Vec<_> = seen
        .iter()
        .map(|c| (c.requested_state, c.duration_minutes, c.invoke_id))
        .collect();
    assert_eq!(
        asked,
        [
            (DccState::DisableInitiation, Some(5), 42),
            (DccState::Enable, None, 42)
        ]
    );
    for context in &seen {
        assert_eq!(context.principal(), None);
        assert_eq!(context.forwarding_peer_principal(), None);
        assert_eq!(context.source_mac.as_slice(), [127, 0, 0, 1, 0xba, 0xc0]);
        let debug = format!("{context:?}");
        assert!(debug.contains("source_mac_len: 6"), "{debug}");
        assert!(!debug.contains("186"), "{debug}");
    }

    // The timer still runs out as before.
    advance(Duration::from_secs(5 * 60)).await;
    tokio::task::yield_now().await;
    assert_eq!(state.get(), DccState::Enable);
}

#[tokio::test(start_paused = true)]
async fn a_panicking_policy_denies() {
    let config = config_with(Arc::new(
        crate::FnPolicy::new().device_communication_control(|_| panic!("policy bug")),
    ));
    for (initial, mode) in [
        (DccState::Enable, EnableDisable::DISABLE_INITIATION),
        (DccState::DisableInitiation, EnableDisable::ENABLE),
    ] {
        let state = comm_state_in(initial);
        let timer = timer();
        assert_denied(dispatch_with_config(&state, &timer, mode, Some(1), &config).await);
        assert_eq!(state.get(), initial);
        assert!(timer.lock().await.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn the_policy_is_asked_only_after_the_password_and_source_checks() {
    use crate::server::{DccSource, DccSourceRestriction};
    let policy = Arc::new(Recording::default());
    policy.allow.store(true, Ordering::Release);
    let mut config = ServerConfig {
        dcc_policy: DccPolicy::RequirePassword,
        dcc_password: Some("required".into()),
        policy: Some(policy.clone()),
        ..Default::default()
    };
    let send = |config: ServerConfig, mode, password: Option<&'static str>| async move {
        let state = comm_state_in(DccState::Enable);
        let response = dispatch_wire(&state, &timer(), mode, None, &config, password, None).await;
        (response, state.get())
    };
    let disable = EnableDisable::DISABLE_INITIATION;

    // A wrong or missing password is refused as before, without asking.
    for password in [Some("wrong"), None] {
        let (response, state) = send(config.clone(), disable, password).await;
        let Apdu::Error(error) = response else {
            panic!("expected Error, got {response:?}")
        };
        assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::SECURITY, ErrorCode::PASSWORD_FAILURE)
        );
        assert_eq!(state, DccState::Enable);
    }
    // The deprecated DISABLE is refused before the policy too.
    let (response, _) = send(config.clone(), EnableDisable::DISABLE, Some("required")).await;
    assert_denied(response);
    assert_eq!(policy.calls(), 0);

    // A source the restriction excludes is refused without asking.
    config.dcc_source_restriction =
        Some(DccSourceRestriction::new(vec![DccSource::Direct(vec![9; 6])]).unwrap());
    let (response, state) = send(config.clone(), disable, Some("required")).await;
    assert_denied(response);
    assert_eq!(state, DccState::Enable);
    assert_eq!(policy.calls(), 0);

    // Past every static check the policy decides: once to allow, once to deny.
    config.dcc_source_restriction = None;
    let (response, state) = send(config.clone(), disable, Some("required")).await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    assert_eq!(state, DccState::DisableInitiation);
    policy.allow.store(false, Ordering::Release);
    let (response, state) = send(config.clone(), disable, Some("required")).await;
    assert_denied(response);
    assert_eq!(state, DccState::Enable);
    assert_eq!(policy.calls(), 2);
    // The password never reaches the policy.
    for context in policy.seen.lock().unwrap().iter() {
        assert!(!format!("{context:?}").contains("required"));
    }
}
