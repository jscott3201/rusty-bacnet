//! Exact identity and explicit-time checks of the production admission helper.

use super::*;
use crate::server::segmented_receive::{
    expire_segmented_requests, segmented_request_admission_error,
};

fn saved_state(invoke_id: u8, now: Instant) -> SegmentedRequestState {
    let first_req = ConfirmedRequestPdu {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: Bytes::from_static(b"retained"),
    };
    let mut receiver = SegmentReceiver::new();
    receiver
        .receive(0, first_req.service_request.clone())
        .unwrap();
    SegmentedRequestState {
        receiver,
        first_req,
        last_activity: now,
        last_progress: now,
        expected_seq: 1,
        initial_sequence_number: 0,
        duplicate_count: 0,
        last_acked_seq: 0,
        window_pos: 0,
        actual_window_size: 1,
        accepted_segments: 1,
    }
}

#[test]
fn request_peer_quota_exact_segkey_projection_identity_matrix() {
    let a = test_mac(1);
    let b = test_mac(2);
    let empty = |network| {
        Some(NpduAddress {
            network,
            mac_address: MacAddr::new(),
        })
    };
    // Named groups are an independent oracle, including invalid fallback
    // partitions. The bool states whether the expected key drops the router MAC.
    let sources = vec![
        ("direct-a", &a, None, false),
        ("direct-b", &b, None, false),
        ("routed", &a, Some(routed_address(400, 10)), true),
        ("routed", &b, Some(routed_address(400, 10)), true),
        ("other-address", &a, Some(routed_address(400, 11)), true),
        ("other-network", &a, Some(routed_address(401, 10)), true),
        ("min-network", &a, Some(routed_address(1, 10)), true),
        ("min-network", &b, Some(routed_address(1, 10)), true),
        ("max-network", &a, Some(routed_address(0xFFFE, 10)), true),
        ("max-network", &b, Some(routed_address(0xFFFE, 10)), true),
        ("zero-net-a", &a, Some(routed_address(0, 10)), false),
        ("zero-net-b", &b, Some(routed_address(0, 10)), false),
        (
            "zero-net-other-address",
            &a,
            Some(routed_address(0, 11)),
            false,
        ),
        ("global-net-a", &a, Some(routed_address(0xFFFF, 10)), false),
        ("global-net-b", &b, Some(routed_address(0xFFFF, 10)), false),
        ("empty-valid-a", &a, empty(400), false),
        ("empty-valid-b", &b, empty(400), false),
        ("empty-zero", &a, empty(0), false),
        ("empty-global", &a, empty(0xFFFF), false),
    ];
    let now = Instant::now();
    for (group, mac, route, drops_router) in &sources {
        let mut receivers = HashMap::new();
        for invoke_id in 0..16 {
            let key = segmented_transaction_key(mac, route.as_ref(), invoke_id);
            let expected_mac = if *drops_router {
                MacAddr::new()
            } else {
                (*mac).clone()
            };
            assert_eq!(key, (expected_mac, route.clone(), invoke_id), "{group}");
            assert_eq!(segmented_request_admission_error(&receivers, &key), None);
            receivers.insert(key, saved_state(invoke_id, now));
        }
        for (query_group, query_mac, query_route, _) in &sources {
            let query = segmented_transaction_key(query_mac, query_route.as_ref(), 200);
            assert_eq!(
                segmented_request_admission_error(&receivers, &query),
                if group == query_group {
                    Some(AbortReason::OUT_OF_RESOURCES)
                } else {
                    None
                },
                "filled {group}, queried {query_group}"
            );
        }
    }
}

#[test]
fn request_peer_quota_repeated_denial_never_touches_live_state() {
    let now = Instant::now();
    let mac = test_mac(1);
    let mut receivers = HashMap::new();
    for invoke_id in 0..16 {
        receivers.insert(
            segmented_transaction_key(&mac, None, invoke_id),
            saved_state(invoke_id, now),
        );
    }
    for invoke_id in 16..=255 {
        let key = segmented_transaction_key(&mac, None, invoke_id);
        assert_eq!(
            segmented_request_admission_error(&receivers, &key),
            Some(AbortReason::OUT_OF_RESOURCES)
        );
        assert!(!receivers.contains_key(&key));
    }
    assert_eq!(receivers.len(), 16);
    for (key, state) in &receivers {
        assert_eq!(state.last_activity, now);
        assert_eq!(state.last_progress, now);
        assert_eq!(state.first_req.invoke_id, key.2);
        assert_eq!(state.first_req.service_request.as_ref(), b"retained");
        assert_eq!(state.receiver.reassemble(1).unwrap(), b"retained");
        assert_eq!(state.accepted_segments, 1);
        assert_eq!(state.expected_seq, 1);
        assert_eq!(state.initial_sequence_number, 0);
        assert_eq!(state.last_acked_seq, 0);
        assert_eq!(state.window_pos, 0);
        assert_eq!(state.duplicate_count, 0);
        assert_eq!(state.actual_window_size, 1);
    }
}

#[test]
fn request_peer_quota_expiry_releases_slot_at_idle_and_progress_boundaries() {
    let start = Instant::now();
    let mac = test_mac(1);
    for (elapsed, progress_expiry) in [(4, false), (16, true)] {
        let now = start + Duration::from_secs(elapsed);
        let recent = now - Duration::from_secs(1);
        let mut receivers = HashMap::new();
        for invoke_id in 0..16 {
            receivers.insert(
                segmented_transaction_key(&mac, None, invoke_id),
                saved_state(invoke_id, recent),
            );
        }
        let stale_key = segmented_transaction_key(&mac, None, 0);
        let stale = receivers.get_mut(&stale_key).unwrap();
        if progress_expiry {
            stale.last_progress = start;
        } else {
            stale.last_activity = start;
        }
        let query = segmented_transaction_key(&mac, None, 200);
        expire_segmented_requests(&mut receivers, now - Duration::from_nanos(1));
        assert_eq!(
            segmented_request_admission_error(&receivers, &query),
            Some(AbortReason::OUT_OF_RESOURCES)
        );
        expire_segmented_requests(&mut receivers, now);
        assert!(!receivers.contains_key(&stale_key));
        assert_eq!(receivers.len(), 15);
        assert_eq!(segmented_request_admission_error(&receivers, &query), None);
        expire_segmented_requests(&mut receivers, now);
        assert_eq!(receivers.len(), 15);
        receivers.insert(query, saved_state(200, now));
        assert_eq!(
            segmented_request_admission_error(
                &receivers,
                &segmented_transaction_key(&mac, None, 201)
            ),
            Some(AbortReason::OUT_OF_RESOURCES)
        );
    }
}
