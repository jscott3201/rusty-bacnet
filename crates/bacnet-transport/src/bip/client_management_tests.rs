use super::*;
use bacnet_bip_core::node::Operation;

async fn peer() -> (UdpSocket, [u8; 6]) {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    (peer, target)
}

#[tokio::test]
async fn bvlc_cancelled_request_releases_slot_for_the_next_request() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let mut packet = [0; 64];
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        tokio::select! {
            result = &mut request => panic!("request finished before a response: {result:?}"),
            received = peer.recv_from(&mut packet) => { received.unwrap(); }
        }
        // Dropping a sent request must release the one outstanding slot.
    }
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        let (_, source) = tokio::select! {
            result = &mut request => panic!("cancelled request stranded the slot: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        peer.send_to(&[0x81, 0x03, 0x00, 0x04], source)
            .await
            .unwrap();
        assert!(request.await.unwrap().is_empty());
    }
    let counters = transport.bvlc_client_snapshot().read_bdt;
    assert_eq!(
        (counters.sent, counters.acknowledgements, counters.timeouts),
        (2, 1, 0)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_ttl_one_renews_before_the_advertised_ttl() {
    let (peer, target) = peer().await;
    let (_, port) = decode_bip_mac(&target).unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: port,
        renewal_interval: None,
        ttl: 1,
    });
    let _rx = transport.start().await.unwrap();
    let mut packet = [0; 64];
    let (len, source) = peer.recv_from(&mut packet).await.unwrap();
    assert_eq!(&packet[..len], &[0x81, 0x05, 0, 6, 0, 1]);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    let (len, _) = tokio::time::timeout(Duration::from_millis(900), peer.recv_from(&mut packet))
        .await
        .expect("TTL1 must schedule a renewal before one second")
        .unwrap();
    assert_eq!(&packet[..len], &[0x81, 0x05, 0, 6, 0, 1]);
    transport.stop().await.unwrap();
}

fn message(function: BvlcFunction, payload: &[u8]) -> BvllMessage {
    BvllMessage {
        function,
        payload: bytes::Bytes::copy_from_slice(payload),
    }
}

/// The peer a test exchanges with.
const TARGET: ([u8; 4], u16) = ([127, 0, 0, 1], 47808);

fn target() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(TARGET.0), TARGET.1)
}

/// A normal node at 192.0.2.10 that sends nowhere, for exchanges driven by
/// hand: [`answer`] feeds it a response.
async fn node_ctx() -> RecvContext {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let socket = Arc::new(BipSocket::recording(socket, send_sink::SendSink::default()));
    let (npdu_tx, _) = mpsc::channel(1);
    RecvContext::for_test(
        socket,
        npdu_tx,
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 47808),
        SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808),
        None,
    )
}

/// `msg` arrives from `source` by unicast.
async fn answer(ctx: &RecvContext, msg: &BvllMessage, source: ([u8; 4], u16)) {
    handle_bvll_message(msg, source, Delivery::Unicast, ctx).await;
}

fn register(ttl: u16) -> Bvll<'static> {
    Bvll::RegisterForeignDevice { ttl }
}

const THREE_SECONDS: Duration = Duration::from_secs(3);

#[tokio::test]
async fn bvlc_early_response_does_not_count_an_unsent_request() {
    let ctx = node_ctx().await;
    let (guard, _rx) = ctx
        .node
        .begin(target(), register(60), THREE_SECONDS)
        .unwrap();
    // The receive worker can run after admission, before the sender returns
    // from its UDP send. A local send failure/cancellation must not leave a
    // falsely counted receipt from that interval.
    answer(&ctx, &message(BvlcFunction::BVLC_RESULT, &[0, 0]), TARGET).await;
    let snapshot = ctx.node.snapshot();
    assert_eq!(snapshot.register_foreign_device.sent, 0);
    assert_eq!(
        snapshot.foreign_registration.accepted, 0,
        "an early response must wait for confirmed local send completion"
    );
    drop(guard);
    assert_eq!(ctx.node.snapshot().register_foreign_device.results, 0);
}

#[tokio::test]
async fn bvlc_snapshot_counts_each_public_management_helper() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    for (request_function, response) in [
        (1, vec![0x81, 0, 0, 6, 0, 0]),
        (2, vec![0x81, 3, 0, 4]),
        (5, vec![0x81, 0, 0, 6, 0, 0]),
        (6, vec![0x81, 7, 0, 4]),
        (8, vec![0x81, 0, 0, 6, 0, 0]),
    ] {
        let request = async {
            match request_function {
                1 => transport.write_bdt(&target, &[]).await.map(|_| ()),
                2 => transport.read_bdt(&target).await.map(|_| ()),
                5 => transport
                    .register_foreign_device_bvlc(&target, 55)
                    .await
                    .map(|_| ()),
                6 => transport.read_fdt(&target).await.map(|_| ()),
                8 => transport
                    .delete_fdt_entry(&target, [10, 0, 0, 2], 47808)
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            }
        };
        tokio::pin!(request);
        let mut packet = [0; 128];
        let (len, source) = tokio::select! {
            result = &mut request => panic!("request finished before peer response: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..2], &[0x81, request_function]);
        if request_function == 5 {
            assert_eq!(&packet[4..len], &[0, 55]);
        }
        peer.send_to(&response, source).await.unwrap();
        request.await.unwrap();
    }
    let snapshot = transport.bvlc_client_snapshot();
    for counters in [snapshot.read_bdt, snapshot.read_fdt] {
        assert_eq!(
            (counters.sent, counters.acknowledgements, counters.results),
            (1, 1, 0)
        );
    }
    for counters in [
        snapshot.write_bdt,
        snapshot.delete_fdt_entry,
        snapshot.register_foreign_device,
    ] {
        assert_eq!(
            (counters.sent, counters.acknowledgements, counters.results),
            (1, 0, 1)
        );
        assert_eq!(
            counters.last_result,
            Some(BvlcResultCode::SUCCESSFUL_COMPLETION)
        );
    }
    assert_eq!(snapshot.foreign_registration.attempts, 1);
    assert_eq!(snapshot.foreign_registration.accepted, 1);
    assert_eq!(snapshot.foreign_registration.last_ttl, Some(55));
    assert_eq!(snapshot.foreign_registration.next_attempt_in, None);
    assert_eq!(
        transport.management_counters(),
        ManagementCounters::default()
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_operation_specific_results_and_malformed_frames_do_not_steal_requests() {
    let ctx = node_ctx().await;
    let (guard, mut rx) = ctx
        .node
        .begin(target(), register(60), THREE_SECONDS)
        .unwrap();
    guard.sent();
    for (source, code) in [
        (([127, 0, 0, 2], 47808), 0), // wrong IP
        (([127, 0, 0, 1], 47809), 0), // wrong port
        (TARGET, 0x60),               // DBTN-NAK is unrelated to registration
        (TARGET, 0x20),               // Read-BDT-NAK is also unrelated
        (TARGET, 0x1234),             // unknown codes stay observable but unclassified
    ] {
        let result = message(BvlcFunction::BVLC_RESULT, &u16::to_be_bytes(code));
        answer(&ctx, &result, source).await;
        assert!(rx.try_recv().is_err());
    }
    for payload in [&[][..], &[0][..], &[0, 0, 0][..]] {
        answer(&ctx, &message(BvlcFunction::BVLC_RESULT, payload), TARGET).await;
        assert!(rx.try_recv().is_err());
    }
    let wrong_ack = message(BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, &[]);
    answer(&ctx, &wrong_ack, TARGET).await;
    let before = ctx.node.snapshot();
    assert_eq!(before.register_foreign_device.results, 0);
    assert_eq!(before.foreign_registration.accepted, 0);
    assert_eq!(before.foreign_registration.refused, 0);
    assert_eq!(before.unmatched_responses, 6);
    assert_eq!(before.last_unmatched_result.unwrap().to_raw(), 0x1234);
    assert_eq!(before.register_foreign_device.malformed_responses, 3);
    answer(
        &ctx,
        &message(BvlcFunction::BVLC_RESULT, &[0, 0x30]),
        TARGET,
    )
    .await;
    assert_eq!(
        decode_bvlc_result_code(&rx.await.unwrap().unwrap()).unwrap(),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(ctx.node.snapshot().foreign_registration.refused, 1);

    // Each own NAK is a matched result; another operation's NAK is not.
    let rows = [codec::BdtEntry::from(&BdtEntry {
        ip: [10, 0, 0, 1],
        port: 47808,
        broadcast_mask: [255; 4],
    })];
    let entry = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 2), 47808);
    for (request, operation, by_ack, own_nak) in [
        (
            Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&rows)),
            Operation::WriteBdt,
            false,
            0x10,
        ),
        (
            Bvll::ReadBroadcastDistributionTable,
            Operation::ReadBdt,
            true,
            0x20,
        ),
        (Bvll::ReadForeignDeviceTable, Operation::ReadFdt, true, 0x40),
        (
            Bvll::DeleteForeignDeviceTableEntry(entry),
            Operation::DeleteFdtEntry,
            false,
            0x50,
        ),
    ] {
        let (guard, mut rx) = ctx.node.begin(target(), request, THREE_SECONDS).unwrap();
        guard.sent();
        answer(
            &ctx,
            &message(BvlcFunction::BVLC_RESULT, &[0, 0x30]),
            TARGET,
        )
        .await;
        assert!(rx.try_recv().is_err());
        if by_ack {
            answer(&ctx, &message(BvlcFunction::BVLC_RESULT, &[0, 0]), TARGET).await;
            assert!(rx.try_recv().is_err());
        }
        let own = message(BvlcFunction::BVLC_RESULT, &[0, own_nak]);
        answer(&ctx, &own, TARGET).await;
        rx.await.unwrap().unwrap();
        let mut snapshot = ctx.node.snapshot();
        assert_eq!(snapshot.counters_mut(operation).results, 1);
        assert_eq!(
            snapshot
                .counters_mut(operation)
                .last_result
                .unwrap()
                .to_raw(),
            u16::from(own_nak)
        );
    }
}

#[tokio::test]
async fn bvlc_malformed_ack_returns_decode_error_without_counting_an_ack() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (_, source) = tokio::select! {
            result = &mut request => panic!("early response: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        peer.send_to(&[0x81, 3, 0, 5, 0], source).await.unwrap();
        assert!(matches!(request.await, Err(Error::Decoding { .. })));
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.read_bdt.sent, 1);
    assert_eq!(snapshot.read_bdt.acknowledgements, 0);
    assert_eq!(snapshot.read_bdt.malformed_responses, 1);
    assert_eq!(snapshot.malformed_responses, 1);
    transport.stop().await.unwrap();
}

/// A request a success answers is busy while another is in flight to the
/// same peer, since a success couldn't tell them apart; it times out on its
/// own; and a success after that is a late result, not an acceptance. A
/// table read to the same peer meanwhile runs beside it (#1622).
#[tokio::test]
async fn bvlc_busy_timeout_and_late_result_are_distinct_observations() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    {
        let request = transport.register_foreign_device_bvlc(&target, 40);
        tokio::pin!(request);
        let mut packet = [0; 64];
        tokio::select! {
            result = &mut request => panic!("early response: {result:?}"),
            received = peer.recv_from(&mut packet) => { received.unwrap(); }
        }
        let busy = transport.write_bdt(&target, &[]).await.unwrap_err();
        assert!(busy.to_string().contains("already in flight"), "{busy}");
        {
            let read = transport.read_fdt(&target);
            tokio::pin!(read);
            let (len, source) = tokio::select! {
                result = &mut read => panic!("early response: {result:?}"),
                received = peer.recv_from(&mut packet) => received.unwrap(),
            };
            assert_eq!(&packet[..len], &[0x81, 6, 0, 4]);
            peer.send_to(&[0x81, 7, 0, 4], source).await.unwrap();
            assert!(read.await.unwrap().is_empty());
        }
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(matches!(request.await, Err(Error::Timeout(_))));
        tokio::time::resume();
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!((snapshot.write_bdt.sent, snapshot.write_bdt.busy), (0, 1));
    assert_eq!(
        (snapshot.read_fdt.sent, snapshot.read_fdt.acknowledgements),
        (1, 1)
    );
    assert_eq!(
        (
            snapshot.register_foreign_device.sent,
            snapshot.register_foreign_device.timeouts
        ),
        (1, 1)
    );
    assert_eq!(
        snapshot.foreign_registration.last_outcome,
        Some(ForeignRegistrationOutcome::TimedOut)
    );
    // A late success with no pending request is observable, not acceptance.
    let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, transport.port);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], to).await.unwrap();
    let late = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = transport.bvlc_client_snapshot();
            if snapshot.unmatched_responses == 1 {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the late result is counted");
    assert_eq!(late.foreign_registration.accepted, 0);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_encode_and_send_errors_release_slot_without_fabricated_timeouts() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let entries = vec![
        BdtEntry {
            ip: [10, 0, 0, 1],
            port: 47808,
            broadcast_mask: [255; 4]
        };
        6554
    ];
    assert!(matches!(
        transport.write_bdt(&target, &entries).await,
        Err(Error::Encoding(_))
    ));
    // A broadcast without socket permission fails locally; UDP port zero
    // does not provide the same send-error behavior on every platform.
    transport
        .socket
        .as_ref()
        .unwrap()
        .set_broadcast(false)
        .unwrap();
    let denied_broadcast = encode_bip_mac(
        Ipv4Addr::BROADCAST.octets(),
        peer.local_addr().unwrap().port(),
    );
    let error = transport.read_bdt(&denied_broadcast).await.unwrap_err();
    assert!(matches!(error, Error::Transport(_)), "{error:?}");
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (_, source) = tokio::select! {
            result = &mut request => panic!("failed request stranded the slot: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        peer.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
        request.await.unwrap();
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(
        (
            snapshot.write_bdt.sent,
            snapshot.write_bdt.local_errors,
            snapshot.write_bdt.timeouts
        ),
        (0, 1, 0)
    );
    assert_eq!(
        (
            snapshot.read_bdt.sent,
            snapshot.read_bdt.local_errors,
            snapshot.read_bdt.timeouts
        ),
        (1, 1, 0)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_stale_guard_cannot_clear_new_request_after_response_or_restart() {
    for restart in [false, true] {
        let mut ctx = node_ctx().await;
        let (old, old_rx) = ctx
            .node
            .begin(target(), register(60), THREE_SECONDS)
            .unwrap();
        old.sent();
        if restart {
            ctx.node.stop();
            assert!(old_rx.await.is_err());
            ctx = node_ctx().await;
        } else {
            answer(&ctx, &message(BvlcFunction::BVLC_RESULT, &[0, 0]), TARGET).await;
            old_rx.await.unwrap().unwrap();
        }
        let (new, new_rx) = ctx
            .node
            .begin(target(), register(90), THREE_SECONDS)
            .unwrap();
        new.sent();
        drop(old);
        answer(&ctx, &message(BvlcFunction::BVLC_RESULT, &[0, 0]), TARGET).await;
        new_rx.await.unwrap().unwrap();
        assert_eq!(ctx.node.snapshot().foreign_registration.last_ttl, Some(90));
        assert_eq!(
            ctx.node.snapshot().foreign_registration.last_outcome,
            Some(ForeignRegistrationOutcome::Accepted)
        );
    }
}

#[tokio::test]
async fn bvlc_early_response_is_delivered_after_successful_local_send() {
    let ctx = node_ctx().await;
    let (guard, mut rx) = ctx
        .node
        .begin(target(), register(60), THREE_SECONDS)
        .unwrap();
    answer(&ctx, &message(BvlcFunction::BVLC_RESULT, &[0, 0]), TARGET).await;
    assert!(matches!(
        rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(ctx.node.snapshot().register_foreign_device.results, 0);
    guard.sent();
    assert_eq!(
        decode_bvlc_result_code(&rx.await.unwrap().unwrap()).unwrap(),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let snapshot = ctx.node.snapshot();
    assert_eq!(
        (
            snapshot.register_foreign_device.sent,
            snapshot.register_foreign_device.results
        ),
        (1, 1)
    );
    assert_eq!(snapshot.foreign_registration.accepted, 1);
}

#[tokio::test]
async fn bvlc_manual_zero_ttl_is_a_one_shot_wire_request() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    {
        let request = transport.register_foreign_device_bvlc(&target, 0);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (len, source) = tokio::select! {
            result = &mut request => panic!("zero TTL was blocked locally: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 0]);
        peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
        assert_eq!(
            request.await.unwrap(),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    assert_eq!(
        transport
            .bvlc_client_snapshot()
            .foreign_registration
            .last_ttl,
        Some(0)
    );
    assert_eq!(
        transport
            .bvlc_client_snapshot()
            .foreign_registration
            .next_attempt_in,
        None
    );
    transport.stop().await.unwrap();
}

/// A table ACK the codec refuses, or one longer than the table limit,
/// fails the request waiting for it with that error, and counts as a
/// malformed response, not an acknowledgement.
#[tokio::test]
async fn a_malformed_or_oversized_ack_fails_the_request_it_answers() {
    let ctx = node_ctx().await;
    let (guard, rx) = ctx
        .node
        .begin(
            target(),
            Bvll::ReadBroadcastDistributionTable,
            THREE_SECONDS,
        )
        .unwrap();
    guard.sent();
    let short = message(BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK, &[0]);
    answer(&ctx, &short, TARGET).await;
    let error = rx.await.unwrap().unwrap_err();
    assert!(matches!(error, Error::Decoding { .. }), "{error:?}");

    let (guard, rx) = ctx
        .node
        .begin(target(), Bvll::ReadForeignDeviceTable, THREE_SECONDS)
        .unwrap();
    guard.sent();
    let row = [192, 0, 2, 40, 0xBA, 0xC0, 0, 60, 0, 90];
    let rows = row.repeat(BbmdState::MAX_FDT_ENTRIES + 1);
    let long = message(BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, &rows);
    answer(&ctx, &long, TARGET).await;
    let error = rx.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("exceeds maximum"), "{error}");

    let snapshot = ctx.node.snapshot();
    assert_eq!(snapshot.malformed_responses, 2);
    for counters in [snapshot.read_bdt, snapshot.read_fdt] {
        assert_eq!(
            (
                counters.sent,
                counters.acknowledgements,
                counters.malformed_responses
            ),
            (1, 0, 1)
        );
    }
}

/// A request still in flight when the transport stops fails, and giving it
/// up afterwards leaves the cleared status as it is.
#[tokio::test]
async fn a_request_given_up_after_stop_leaves_the_status_clear() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let node = Arc::clone(transport.node.as_ref().unwrap());
    let request = node.request(&target, register(60), THREE_SECONDS);
    tokio::pin!(request);
    let mut packet = [0; 64];
    tokio::select! {
        result = &mut request => panic!("request finished before a response: {result:?}"),
        received = peer.recv_from(&mut packet) => { received.unwrap(); }
    }
    transport.stop().await.unwrap();
    let error = request.await.unwrap_err();
    assert!(error.to_string().contains("channel dropped"), "{error}");
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.foreign_registration.last_outcome, None);
    assert_eq!(snapshot.foreign_registration.attempts, 1);
    assert_eq!(snapshot.register_foreign_device.sent, 1);
}

/// What a call racing `stop()` can do to a manual registration in flight:
/// give it up, or match the BBMD's success to it.
#[derive(Clone, Copy, Debug)]
enum Race {
    GiveUp,
    Answer,
}

/// A call that lands while the transport stops, after its node stopped and
/// before the client status is cleared, changes nothing: the status stays
/// clear, as when the old client reset and refused counts under one lock.
#[tokio::test]
async fn a_call_racing_stop_leaves_the_status_clear() {
    for race in [Race::GiveUp, Race::Answer] {
        let (_peer, target) = peer().await;
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _rx = transport.start().await.unwrap();
        let node = Arc::clone(transport.node.as_ref().unwrap());
        let peer = codec::address_from_slice(&target).unwrap();
        let (exchange, _response) = node.begin(peer, register(60), THREE_SECONDS).unwrap();
        exchange.sent();
        let pending = transport.bvlc_client_snapshot().foreign_registration;
        assert_eq!(
            pending.last_outcome,
            Some(ForeignRegistrationOutcome::Pending)
        );
        transport.between_stops = Some(Box::new(move || match race {
            Race::GiveUp => drop(exchange),
            Race::Answer => {
                use bacnet_bip_core::node::{Addressed, Datagram, Permissive, Received};
                // The BBMD's success, as the node's link reads it from the
                // peer by unicast.
                let frame = [0x81, 0x00, 0x00, 0x06, 0x00, 0x00];
                let to = Addressed {
                    node: true,
                    broadcast: false,
                };
                let datagram = Datagram::new(&frame, peer, Some(to));
                let received =
                    node.link()
                        .receive(&datagram, bacnet_bip_core::Instant::ZERO, &mut Permissive);
                let Received::Response(response) = received else {
                    panic!("{received:?}");
                };
                assert_eq!(
                    response.message,
                    Ok(Bvll::BvlcResult(BvlcResultCode::SUCCESSFUL_COMPLETION))
                );
                node.resolve(response, &frame, None);
            }
        }));
        transport.stop().await.unwrap();
        let stopped = transport.bvlc_client_snapshot();
        let registration = stopped.foreign_registration;
        assert_eq!(registration.last_outcome, None, "{race:?}");
        assert_eq!(registration.last_ttl, None, "{race:?}");
        assert_eq!(registration.accepted, 0, "{race:?}");
        assert_eq!(
            stopped.register_foreign_device.last_result, None,
            "{race:?}"
        );
    }
}
