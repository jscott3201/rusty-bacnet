//! The server's side of expiry: every due transfer is out of the table
//! before its Abort awaits, and a failed Abort leaves the rest to go. The
//! boundaries themselves are the shared receiver's (#2742), tested with it in
//! `bacnet_endpoint_core::segmented_request`.

use super::*;
use bacnet_endpoint_core::segmented_request::{Receipt, Segment, SegmentedRequests};
use bacnet_network::response_route::ResponseRoute;

#[tokio::test]
async fn progress_expiry_empties_the_table_before_its_aborts_and_survives_a_failed_send() {
    let start = Instant::now();
    let now = start + Duration::from_secs(16);
    // Four segment timeouts of 5 s: at 16 s silence isn't due, progress is.
    let mut table = SegmentedRequests::new(Duration::from_secs(5));
    let route = ResponseRoute::unverified();
    let source = test_mac(1);
    for invoke_id in 0..3 {
        let first = ConfirmedRequestPdu {
            segmented: true,
            more_follows: true,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: Some(0),
            proposed_window_size: Some(3),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::from_static(b"first"),
        };
        let segment = Segment {
            source_mac: source.as_slice(),
            source_network: None,
            route: &route,
            request: &first,
        };
        assert!(matches!(table.receive(segment, start), Receipt::Ack(_)));
    }
    let expired = table.expire(now);
    assert_eq!(expired.len(), 3);
    assert_eq!(
        (table.len(), table.saved_bytes()),
        (0, 0),
        "every transfer is out before any Abort awaits"
    );
    let (network, sent, control) = blocking_first_send_network();
    control.fail_next_send();
    let aborting = BACnetServer::abort_expired(&network, expired);
    tokio::pin!(aborting);
    tokio::select! {
        () = &mut aborting => panic!("the first Abort should block"),
        () = control.wait_blocked() => {}
    }
    control.release_sends(1);
    aborting.await;
    assert_eq!(
        sent_count(&sent),
        3,
        "a failed first Abort doesn't hold back the others"
    );
    for index in 0..3 {
        assert_eq!(abort_reason(&sent, index), AbortReason::OTHER);
        assert_eq!(sent_link_destination(&sent, index), source);
    }
}
