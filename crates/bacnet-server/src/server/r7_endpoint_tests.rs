//! Endpoint responses preserve the same router and data-link ownership.
use super::*;
use crate::server::requests::endpoint_responder::EndpointResponder;
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_types::enums::EngineeringUnits;

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(7, "input", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    db
}
#[tokio::test]
async fn r7_endpoint_queued_and_prompt_replies_keep_target_and_reject_retired_context() {
    let mut routed = Routed::new().await;
    let (port, _keep_open) = TestTransport::inbound(8);
    let mut endpoint = EndpointIngress::new(port, 4);
    let ingress = endpoint.start().await.unwrap();
    let responder =
        EndpointResponder::new(Arc::new(RwLock::new(database())), ingress.egress.clone());
    let received = routed.receive(2000, &read(7), None).await;
    assert!(responder.handle(received).await.unwrap());
    assert!(matches!(
        routed.response(2000, 7).await.0,
        Apdu::ComplexAck(_)
    ));
    let (tx, rx) = oneshot::channel();
    let received = routed.receive(3000, &read(8), Some(tx)).await;
    assert!(responder.handle(received).await.unwrap());
    assert!(matches!(
        decode_response(rx.await.unwrap(), 3000, 8).0,
        Apdu::ComplexAck(_)
    ));
    let route = routed.receive(2000, &read(9), None).await.response_route();
    let queued = ingress
        .egress
        .admit_response_apdu(
            vec![0x20, 9, 12],
            MacAddr::from_slice(&[9]),
            Some(address(9000, &[7])),
            route.clone(),
        )
        .unwrap();
    drop(queued);
    tokio::task::yield_now().await;
    assert!(
        routed.wire.try_recv().is_err(),
        "cancelled queued endpoint response emits nothing"
    );
    let (tx, rx) = oneshot::channel();
    let received = routed.receive(2000, &read(10), Some(tx)).await;
    routed.router.stop().await;
    assert!(responder.handle(received).await.is_err());
    assert!(rx.await.is_err());
    let queued = ingress
        .egress
        .admit_response_apdu(
            vec![0x20, 9, 12],
            MacAddr::from_slice(&[9]),
            Some(address(9000, &[7])),
            route,
        )
        .unwrap();
    assert!(queued.complete().await.result.is_err());
    endpoint.stop().await.unwrap();
    assert!(routed.wire.try_recv().is_err());
}

#[tokio::test]
async fn r7_endpoint_retirement_during_read_prevents_prompt_der_handoff() {
    use std::future::Future;
    for close_responder in [false, true] {
        let mut routed = Routed::new().await;
        let (port, _keep_open) = TestTransport::inbound(8);
        let mut endpoint = EndpointIngress::new(port, 4);
        let ingress = endpoint.start().await.unwrap();
        let db = Arc::new(RwLock::new(database()));
        let responder = EndpointResponder::new(db.clone(), ingress.egress);
        let held = db.write().await;
        let (tx, rx) = oneshot::channel();
        let received = routed.receive(2000, &read(11), Some(tx)).await;
        let mut handle = Box::pin(responder.handle(received));
        assert!(handle
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            .is_pending());
        if close_responder {
            responder.close();
        } else {
            endpoint.stop().await.unwrap();
        }
        drop(held);
        assert!(handle.await.is_err());
        assert!(rx.await.is_err());
        assert!(routed.wire.try_recv().is_err());
        if close_responder {
            endpoint.stop().await.unwrap();
        }
        routed.router.stop().await;
    }
}

#[tokio::test]
async fn r7_server_scope_retirement_and_cancel_release_pending_der_owner_without_reply() {
    for cancel in [false, true] {
        let mut routed = Routed::new().await;
        let context = context(database(), ServerConfig::default());
        let held = context.services.db.write().await;
        let (tx, rx) = oneshot::channel();
        let received = routed.receive(2000, &read(12), Some(tx)).await;
        dispatch(&context, received).await;
        if cancel {
            context.request_tasks.close();
        } else {
            context.services.network.seal_responses();
        }
        drop(held);
        settled(&context).await;
        assert!(rx.await.is_err());
        assert!(routed.wire.try_recv().is_err());
        assert_eq!(context.request_tasks.counters().confirmed_active, 0);
        routed.router.stop().await;
    }
}
