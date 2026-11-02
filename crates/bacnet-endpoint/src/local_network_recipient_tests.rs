//! A provisioned recipient Address naming the session's network starts
//! unresolved while the session has not learned its number, as a Device
//! with no binding does, and resolves once it has (#1461). It resolves
//! against the number in force at each operation, so a replaced number
//! leaves it unresolved, and a recipient change then goes ahead with no
//! copy for it.
use super::*;
use bacnet_types::constructed::BACnetAuditNotification;
use bacnet_types::enums::{PropertyIdentifier, Reliability};
use bacnet_types::primitives::PropertyValue;

async fn reliability(endpoint: &Endpoint) -> Reliability {
    let db = endpoint.session.database.as_ref().unwrap().read().await;
    let PropertyValue::Enumerated(value) = db
        .get(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
    else {
        panic!("Reliability is an enumeration")
    };
    Reliability::from_raw(value)
}

fn direct() -> EndpointApduDestination {
    EndpointApduDestination::Direct {
        destination_mac: mac(PEER),
    }
}

/// Complete a direct read that has no recipient to report to. Nothing else
/// may go out before the next frame a test expects: a stray record would
/// stand in its place.
async fn unreported_read(endpoint: &mut Endpoint) {
    let read = endpoint.read(direct());
    let (route, request) = endpoint.request().await;
    assert_eq!(route, local(PEER));
    endpoint
        .deliver_apdu(ack(&request, 5), mac(PEER), None)
        .await;
    bounded(read).await.unwrap().unwrap();
}

/// The unconfirmed Audit record in `sent`, checked to have gone out by
/// global broadcast: a link broadcast carrying DNET 0xFFFF.
fn broadcast_record(sent: Sent) -> BACnetAuditNotification {
    assert!(sent.link.is_empty(), "sent as a link broadcast");
    let npdu = decode_npdu(sent.npdu).unwrap();
    assert_eq!(npdu.destination.map(|to| to.network), Some(0xFFFF));
    let Apdu::UnconfirmedRequest(pdu) = decode_apdu(npdu.payload).unwrap() else {
        panic!("an unconfirmed notification")
    };
    assert_eq!(
        pdu.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
    );
    let mut request = AuditNotificationRequest::decode(&pdu.service_request).unwrap();
    assert_eq!(request.notifications.len(), 1);
    request.notifications.remove(0)
}

/// The two frames a recipient change sends, the global broadcast (if any)
/// split out from the copies sent to a station.
async fn change_frames(endpoint: &mut Endpoint) -> (Vec<Sent>, Vec<Sent>) {
    let pair = [endpoint.next().await, endpoint.next().await];
    pair.into_iter().partition(|sent| sent.link.is_empty())
}

#[tokio::test(start_paused = true)]
async fn an_address_on_the_network_starts_unresolved_and_resolves_once_learned() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        // Before #1461 the session refused to start with this recipient.
        let mut endpoint = source_reporting_to(role, false, address(THIS_NETWORK, SINK)).await;
        assert_eq!(
            reliability(&endpoint).await,
            Reliability::CONFIGURATION_ERROR,
            "{role:?}"
        );
        unreported_read(&mut endpoint).await;
        assert_eq!(
            reliability(&endpoint).await,
            Reliability::CONFIGURATION_ERROR
        );
        // Learning the number is the next exchange, so no record went out,
        // and health follows the number at once, before any operation.
        endpoint.learn(THIS_NETWORK).await;
        assert_eq!(reliability(&endpoint).await, Reliability::NO_FAULT_DETECTED);
        audited_read(&mut endpoint, direct(), SINK).await;
        // Both recipients resolve: one copy to each, and no broadcast.
        let next = address(THIS_NETWORK, NEW_SINK);
        bounded(endpoint.session.write_audit_recipient(Some(next)))
            .await
            .unwrap();
        let (broadcasts, mut copies) = change_frames(&mut endpoint).await;
        assert!(broadcasts.is_empty(), "{role:?}: no global broadcast");
        copies.sort_by_key(|sent| sent.link.to_vec());
        let [old, new] = <[Sent; 2]>::try_from(copies).ok().unwrap();
        assert_eq!(record(old, SINK), record(new, NEW_SINK));
        audited_read(&mut endpoint, direct(), NEW_SINK).await;
        endpoint.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_replaced_number_unresolves_the_address_and_a_change_still_goes_ahead() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let mut endpoint = source_reporting_to(role, true, address(THIS_NETWORK, SINK)).await;
        audited_read(&mut endpoint, direct(), SINK).await;
        // A later announcement replaces the learned number, and health
        // follows it at once.
        endpoint.learn(REMOTE_NETWORK).await;
        assert_eq!(
            reliability(&endpoint).await,
            Reliability::CONFIGURATION_ERROR,
            "{role:?}"
        );
        unreported_read(&mut endpoint).await;
        // The old address has no route now. The change is not refused for
        // it: the new recipient gets the record, and a global broadcast
        // stands in for the old one's copy (Clause 12.11.66).
        let next = address(REMOTE_NETWORK, NEW_SINK);
        bounded(endpoint.session.write_audit_recipient(Some(next)))
            .await
            .unwrap();
        let (mut broadcasts, mut copies) = change_frames(&mut endpoint).await;
        let change = record(copies.pop().expect("the new recipient's copy"), NEW_SINK);
        assert_eq!(broadcast_record(broadcasts.pop().unwrap()), change);
        assert_eq!(change.operation, AuditOperation::WRITE);
        let mut old = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(&mut old, &address(THIS_NETWORK, SINK))
            .unwrap();
        assert_eq!(change.current_value.as_deref(), Some(&old[..]));
        // The next frame is the read's request: nothing else went out.
        audited_read(&mut endpoint, direct(), NEW_SINK).await;
        assert_eq!(reliability(&endpoint).await, Reliability::NO_FAULT_DETECTED);
        endpoint.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn an_address_off_a_number_known_at_startup_also_starts_unresolved() {
    // A registered port publishes its number before the source starts. An
    // address on another network starts unresolved too: the number may
    // still come to name it.
    let mut endpoint = unstarted_source(
        SessionRole::Both,
        address(REMOTE_NETWORK, SINK),
        Some(THIS_NETWORK),
    );
    endpoint.start(false).await;
    assert_eq!(
        reliability(&endpoint).await,
        Reliability::CONFIGURATION_ERROR
    );
    unreported_read(&mut endpoint).await;
    let next = address(THIS_NETWORK, NEW_SINK);
    bounded(endpoint.session.write_audit_recipient(Some(next)))
        .await
        .unwrap();
    let (mut broadcasts, mut copies) = change_frames(&mut endpoint).await;
    let change = record(copies.pop().unwrap(), NEW_SINK);
    assert_eq!(broadcast_record(broadcasts.pop().unwrap()), change);
    audited_read(&mut endpoint, direct(), NEW_SINK).await;
    endpoint.stop().await;
}
