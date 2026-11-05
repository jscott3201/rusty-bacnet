//! A policy that declares [`ServerPolicy::decides_inline`] is asked on the
//! request task, with no trip to Tokio's blocking pool (#2049). Any other
//! policy is asked on the pool, so one that blocks still can't hold
//! `stop()`; an inline one that blocks holds `stop()` until it returns.
//!
//! [`ServerPolicy::decides_inline`]: crate::ServerPolicy::decides_inline

use super::mutation_tests::{Fixture, apdu, assert_denied, oid, value, wpm};
use super::policy_off_task_tests::{
    Gate, confirmed, dcc, gate, life_safety, send, server, unconfirmed_audit,
};
use super::*;
use crate::audit_notification::UnconfirmedAuditNotificationAuthorizationContext;
use crate::device_communication_control::DeviceCommunicationControlAuthorizationContext;
use crate::life_safety::LifeSafetyOperationAuthorizationContext;
use crate::mutation::MutationAuthorizationContext;
use crate::server::test_transport::TestTransport;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::thread::ThreadId;
use std::time::Duration;
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(10);

/// Allows everything it is asked, reporting the thread each call runs on.
struct Where {
    inline: bool,
    threads: mpsc::UnboundedSender<ThreadId>,
}

impl Where {
    fn allow(&self) -> bool {
        self.threads.send(std::thread::current().id()).unwrap();
        true
    }
}

impl crate::ServerPolicy for Where {
    fn decides_inline(&self) -> bool {
        self.inline
    }
    fn authorize_mutation(&self, _: &MutationAuthorizationContext) -> bool {
        self.allow()
    }
    fn authorize_life_safety_operation(&self, _: &LifeSafetyOperationAuthorizationContext) -> bool {
        self.allow()
    }
    fn authorize_device_communication_control(
        &self,
        _: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        self.allow()
    }
    fn authorize_unconfirmed_audit_notification(
        &self,
        _: &UnconfirmedAuditNotificationAuthorizationContext,
    ) -> bool {
        self.allow()
    }
}

fn description(instance: u32) -> WriteAccessSpecification {
    WriteAccessSpecification {
        object_identifier: oid(ObjectType::BINARY_VALUE, instance),
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            value: value(PropertyValue::CharacterString("written".into())),
            priority: None,
        }],
    }
}

/// A WriteProperty of Binary Value 1's Description.
fn write_property() -> Apdu {
    confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        write_property_request(),
    )
}

fn write_property_request() -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: value(PropertyValue::CharacterString("written".into())),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

/// Each request that reaches a policy, through both of the server's ways
/// of asking, and how many calls it makes.
fn requests() -> [(&'static str, Apdu, usize); 5] {
    [
        ("write_property", write_property(), 1),
        (
            "write_property_multiple",
            confirmed(
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                wpm(vec![description(1), description(2)]),
            ),
            2,
        ),
        ("device_communication_control", dcc(), 1),
        ("life_safety_operation", life_safety(), 1),
        ("unconfirmed_audit_notification", unconfirmed_audit(), 1),
    ]
}

// The current-thread runtime runs every task, each request's included, on
// this thread, and its blocking pool on others.
#[tokio::test]
async fn an_inline_policy_is_asked_on_the_request_task_and_any_other_on_the_pool() {
    let test_thread = std::thread::current().id();
    for inline in [false, true] {
        for (service, request, calls) in requests() {
            let (threads, mut asked_on) = mpsc::unbounded_channel();
            let (mut server, inbound) = server(Where { inline, threads }).await;
            send(&inbound, request).await;
            for _ in 0..calls {
                let thread = tokio::time::timeout(WAIT, asked_on.recv())
                    .await
                    .unwrap_or_else(|_| panic!("{service}: the policy was asked"))
                    .unwrap();
                assert_eq!(
                    thread == test_thread,
                    inline,
                    "{service}: asked on the request task exactly when inline ({inline})"
                );
            }
            server.stop().await.unwrap();
            assert!(asked_on.try_recv().is_err(), "{service}: {calls} calls");
        }
    }
}

/// Blocks each mutation decision on a gate, then allows it and raises
/// `returned`. It keeps the trait's [`decides_inline`] default.
///
/// [`decides_inline`]: crate::ServerPolicy::decides_inline
struct Blocking {
    block: Arc<dyn Fn() + Send + Sync>,
    returned: Arc<AtomicBool>,
}

impl crate::ServerPolicy for Blocking {
    fn authorize_mutation(&self, _: &MutationAuthorizationContext) -> bool {
        (self.block)();
        self.returned.store(true, Ordering::SeqCst);
        true
    }
}

/// `Blocking`, declared inline, against its own rules.
struct Inline(Blocking);

impl crate::ServerPolicy for Inline {
    fn decides_inline(&self) -> bool {
        true
    }
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        self.0.authorize_mutation(context)
    }
}

/// A server whose WriteProperty policy is blocked in its call.
async fn blocked(inline: bool) -> (BACnetServer<TestTransport>, Gate, Arc<AtomicBool>) {
    let mut gate = gate();
    let returned = Arc::new(AtomicBool::new(false));
    let policy = Blocking {
        block: Arc::clone(&gate.block),
        returned: Arc::clone(&returned),
    };
    let (server, inbound) = if inline {
        server(Inline(policy)).await
    } else {
        server(policy).await
    };
    send(&inbound, write_property()).await;
    tokio::time::timeout(WAIT, gate.entered.recv())
        .await
        .expect("the policy was asked")
        .unwrap();
    (server, gate, returned)
}

/// A hand-written policy that keeps the trait's default is asked on the
/// pool: blocked there, it holds neither a worker nor `stop()`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blocking_policy_that_keeps_the_default_still_cannot_hold_stop() {
    let (mut server, gate, returned) = blocked(false).await;
    tokio::time::timeout(WAIT, server.stop())
        .await
        .expect("stop ends while the policy is blocked")
        .unwrap();
    assert!(!returned.load(Ordering::SeqCst), "stopped while it blocked");
    // The abandoned call finishes on its own thread and is ignored.
    gate.release.send(()).unwrap();
}

/// An inline call holds its request task, so `stop()`, which joins every
/// request it admitted, ends only once the call has returned: the bound is
/// the call itself, which is why an inline policy must never block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_once_a_running_inline_call_returns() {
    let (mut server, gate, returned) = blocked(true).await;
    let mut stopping = tokio::spawn(async move { server.stop().await });
    // Inline, stop can't end while the call blocks, so this window can only
    // pass; asked on the pool, stop would end well inside it.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut stopping)
            .await
            .is_err(),
        "stop waits while the call blocks"
    );
    gate.release.send(()).unwrap();
    tokio::time::timeout(WAIT, stopping)
        .await
        .expect("stop ends once the call returns")
        .unwrap()
        .unwrap();
    assert!(returned.load(Ordering::SeqCst), "stop waited for the call");
}

/// A closure that panics on its first call and allows every later one.
fn panics_once<C>() -> impl Fn(&C) -> bool + Send + Sync + 'static {
    let calls = AtomicUsize::new(0);
    move |_| {
        assert_ne!(calls.fetch_add(1, Ordering::SeqCst), 0, "policy panic");
        true
    }
}

/// A panic in an inline policy denies, as it does on the pool, and the
/// request path keeps serving: the next request is asked and allowed. One
/// service through each of the server's ways of asking.
#[tokio::test]
async fn an_inline_policy_that_panics_denies_and_the_next_request_is_served() {
    let mut fixture = Fixture::new(None);
    fixture.config.policy = Some(Arc::new(
        crate::FnPolicy::new()
            .mutation(panics_once())
            .life_safety_operation(panics_once())
            .inline(),
    ));
    let mut life_safety = BytesMut::new();
    bacnet_services::life_safety::LifeSafetyOperationRequest {
        requesting_process_identifier: 7,
        requesting_source: "operator".into(),
        request: bacnet_types::enums::LifeSafetyOperation::SILENCE,
        object_identifier: None,
    }
    .encode(&mut life_safety)
    .unwrap();
    for (service, request) in [
        (
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_property_request(),
        ),
        (
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            life_safety.freeze(),
        ),
    ] {
        let denied = fixture.dispatch(service, request.clone(), 1).await;
        assert_denied(denied.expect("answered"), service, 1);
        let served = fixture.dispatch(service, request, 2).await;
        assert!(
            matches!(apdu(served.expect("answered")), Apdu::SimpleAck(_)),
            "{service:?}: served after the panic"
        );
    }
}
