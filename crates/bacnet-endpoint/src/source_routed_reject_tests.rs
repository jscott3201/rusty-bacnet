//! An audited request a router refuses (#2724): the node's source Audit
//! runs it through the client's one request path, so the write to a
//! network the router has no route to fails at once with the router's
//! reject, and its audit record's Result is the COMMUNICATION code for the
//! reason.
use super::*;

/// A network the router has no route to.
const NOWHERE: u16 = 7;

#[tokio::test(start_paused = true)]
async fn an_audited_write_the_router_refuses_is_recorded_with_the_reason() {
    let config = SessionConfig {
        apdu_timeout_ms: 3_000,
        apdu_retries: 0,
        ..SessionConfig::default()
    };
    let mut site = Site::start_with(
        SessionRole::ClientOnly,
        false,
        &[AuditOperation::WRITE],
        config,
    )
    .await;
    let full = site.session.client().unwrap();
    let station = bacnet_client::client::Station::routed(&ROUTER_NEAR, NOWHERE, &STATION);
    let started = tokio::time::Instant::now();
    let result = full
        .write_property(
            &station,
            oid(ObjectType::ANALOG_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0x44, 0x42, 0x28, 0x00, 0x00],
            None,
        )
        .await;
    let Err(Error::NetworkRejected(reject)) = result else {
        panic!("expected the router's reject, got {result:?}")
    };
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "it waited for a timeout"
    );
    assert_eq!(reject.router.as_slice(), ROUTER_NEAR);
    assert_eq!((reject.network, reject.reason.to_raw()), (NOWHERE, 1));
    assert!(reject.single_frame);
    let (record, _) = notification(&site.receive().await, false);
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(
        record.result,
        Some((ErrorClass::COMMUNICATION, ErrorCode::NOT_ROUTER_TO_DNET))
    );
    drop(full);
    site.stop().await;
}
