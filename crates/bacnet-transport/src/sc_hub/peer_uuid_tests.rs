//! Raw mTLS peers deliberately bypass local startup identity guards.

use super::deadline_test_support::{poll_io, request, ClientWs, TestTls};
use super::heartbeat_test_support::{clients, ClockIo};
use super::*;
use crate::sc_frame::connect_test_support::{
    valid_connect, zero_limits_requests, zero_uuid_requests, InvalidConnect,
};
use std::sync::atomic::{AtomicU16, AtomicUsize};
use std::time::Duration;

struct Peer {
    ws: ClientWs,
    task: JoinHandle<()>,
    deadline: Arc<super::deadlines::ConnectDeadline>,
    active: Arc<AtomicUsize>,
}

impl Peer {
    async fn open(tls: &TestTls, clients: Clients) -> Self {
        let (server, ws, address, accepted) = tls.pair().await;
        let (write, read) = server.split();
        let deadline = Arc::new(super::deadlines::ConnectDeadline::new(
            accepted + Duration::from_secs(5),
        ));
        let active = Arc::new(AtomicUsize::new(0));
        let admission = super::connection::Admission::new(active.clone(), Duration::from_secs(10));
        // Do not retain a test-owned sink: worker completion must release TLS.
        let operation = super::deadlines::serve(
            address,
            ([0x10; 6], [0x10; 16]),
            read,
            Arc::new(Mutex::new(write)),
            clients,
            deadline.clone(),
            || {},
        );
        let task = tokio::spawn(async move {
            let _admission = admission;
            operation.await;
        });
        Self {
            ws,
            task,
            deadline,
            active,
        }
    }

    async fn binary(&mut self) -> Vec<u8> {
        match poll_io(self.ws.next()).await.unwrap().unwrap() {
            Message::Binary(data) => data.to_vec(),
            other => panic!("expected BVLC binary, got {other:?}"),
        }
    }

    async fn rejected(&mut self) {
        poll_io(&mut self.task).await.unwrap();
        assert!(matches!(
            poll_io(self.ws.next()).await,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));
        assert!(!self.deadline.admission_started.load(Ordering::Acquire));
        assert!(!self.deadline.is_committed());
        assert_eq!(self.active.load(Ordering::Acquire), 0);
    }

    async fn close(&mut self) {
        self.ws.close(None).await.unwrap();
        poll_io(&mut self.task).await.unwrap();
        assert_eq!(self.active.load(Ordering::Acquire), 0);
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn zero_uuid_mtls_request_never_reaches_admission() {
    check_invalid_requests_before_admission(zero_uuid_requests()).await;
}

#[tokio::test]
async fn zero_limits_mtls_request_never_reaches_admission() {
    check_invalid_requests_before_admission(zero_limits_requests()).await;
}

async fn check_invalid_requests_before_admission(cases: Vec<InvalidConnect>) {
    let tls = TestTls::new();
    let clients = clients();
    for case in cases {
        let mut peer = Peer::open(&tls, clients.clone()).await;
        peer.ws
            .send(Message::Binary(case.wire.into()))
            .await
            .unwrap();
        if let Some(nak) = case.nak {
            assert_eq!(peer.binary().await, nak, "{}", case.name);
        }
        peer.rejected().await;
        assert!(clients.lock().await.is_empty(), "{}", case.name);
    }
}

#[tokio::test]
async fn zero_uuid_mtls_collision_at_capacity_preserves_live_peers() {
    check_invalid_collision_at_capacity(std::slice::from_ref(&(10..26))).await;
}

#[tokio::test]
async fn zero_limits_mtls_collision_at_capacity_preserves_live_peers() {
    check_invalid_collision_at_capacity(&[26..28, 28..30, 26..30]).await;
}

async fn check_invalid_collision_at_capacity(fields: &[std::ops::Range<usize>]) {
    let tls = TestTls::new();
    let clients = clients();
    let mut peers = Vec::new();
    // Fill the actual 256-client registry with distinct nonzero identities.
    for index in 1u16..=256 {
        let mut vmac = [2, 0, 0, 0, 0, 0];
        vmac[4..].copy_from_slice(&index.to_be_bytes());
        let mut uuid = [0; 16];
        uuid[14..].copy_from_slice(&index.to_be_bytes());
        let mut peer = Peer::open(&tls, clients.clone()).await;
        peer.ws.send(request(vmac, uuid)).await.unwrap();
        assert_eq!(peer.binary().await[0..4], [7, 0, 0x22, 0x33]);
        peers.push(peer);
    }
    let owner = [2, 0, 0, 0, 0, 1];
    let before = {
        let map = clients.lock().await;
        map.iter()
            .map(|(vmac, client)| {
                (
                    *vmac,
                    client.sink.clone(),
                    client.device_uuid,
                    client.heartbeat,
                    client.last_activity.load(Ordering::Acquire),
                )
            })
            .collect::<Vec<_>>()
    };
    for vmac in [owner, [0x10; 6], [0x42; 6]] {
        for field in fields {
            let mut bad = Peer::open(&tls, clients.clone()).await;
            let mut wire = valid_connect(6, vmac);
            // Spoof the incumbent UUID, including from a different proposed VMAC.
            wire[10..26].fill(0);
            wire[25] = 1;
            wire[field.clone()].fill(0);
            bad.ws.send(Message::Binary(wire.into())).await.unwrap();
            // Admission error beats replacement, Duplicate-VMAC and capacity NAKs.
            assert_eq!(bad.binary().await, [0, 0, 0x22, 0x33, 6, 1, 0, 0, 7, 0, 80]);
            bad.rejected().await;
            let map = clients.lock().await;
            assert_eq!(map.len(), 256);
            for (vmac, sink, uuid, heartbeat, activity) in &before {
                let client = map.get(vmac).unwrap();
                assert!(Arc::ptr_eq(&client.sink, sink));
                assert_eq!(client.device_uuid, *uuid);
                assert_eq!(client.heartbeat, *heartbeat);
                assert_eq!(client.last_activity.load(Ordering::Acquire), *activity);
                assert_eq!((client.max_bvlc, client.max_npdu), (8192, 4096));
                assert!(!client.closed.load(Ordering::Acquire));
            }
        }
    }
    // A surviving connection still relays a real NPDU to another live peer.
    peers[0]
        .ws
        .send(Message::Binary(
            vec![1, 4, 0, 9, 2, 0, 0, 0, 0, 2, 1, 0].into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        peers[1].binary().await,
        [1, 8, 0, 9, 2, 0, 0, 0, 0, 1, 1, 0]
    );
    drop(before); // release snapshot sink references before worker cleanup
    for peer in &mut peers {
        peer.close().await;
    }
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn zero_uuid_mtls_repeat_flood_preserves_activity_probe_and_registration() {
    check_invalid_repeat_flood(zero_uuid_requests()).await;
}

#[tokio::test]
async fn zero_limits_mtls_repeat_flood_preserves_activity_probe_and_registration() {
    check_invalid_repeat_flood(zero_limits_requests()).await;
}

async fn check_invalid_repeat_flood(cases: Vec<InvalidConnect>) {
    let tls = TestTls::new();
    let clients = clients();
    let vmac = [0x22; 6];
    let mut peer = Peer::open(&tls, clients.clone()).await;
    peer.ws.send(request(vmac, [0x22; 16])).await.unwrap();
    assert_eq!(peer.binary().await[0], 7);
    clients
        .lock()
        .await
        .get(&vmac)
        .unwrap()
        .last_activity
        .store(0, Ordering::Release);
    super::heartbeat::sweep(
        &clients,
        &AtomicU16::new(0x7788),
        &ClockIo(AtomicU64::new(100)),
    )
    .await;
    assert_eq!(peer.binary().await, [0x0a, 0, 0x77, 0x88]);
    let (sink, heartbeat, activity, closed, notify) = {
        let map = clients.lock().await;
        let client = map.get(&vmac).unwrap();
        (
            client.sink.clone(),
            client.heartbeat,
            client.last_activity.clone(),
            client.closed.clone(),
            client.close_notify.clone(),
        )
    };
    for _ in 0..10 {
        for case in &cases {
            peer.ws
                .send(Message::Binary(case.wire.clone().into()))
                .await
                .unwrap();
            if let Some(nak) = &case.nak {
                assert_eq!(&peer.binary().await, nak);
            } else {
                // Eligible nil request is a non-activity processing barrier.
                peer.ws.send(request(vmac, [0; 16])).await.unwrap();
                assert_eq!(
                    peer.binary().await,
                    [0, 0, 0x22, 0x33, 6, 1, 0, 0, 7, 0, 80]
                );
            }
            let map = clients.lock().await;
            assert_eq!(map.len(), 1);
            let client = map.get(&vmac).unwrap();
            assert!(Arc::ptr_eq(&client.sink, &sink));
            assert!(Arc::ptr_eq(&client.last_activity, &activity));
            assert!(Arc::ptr_eq(&client.closed, &closed));
            assert!(Arc::ptr_eq(&client.close_notify, &notify));
            assert_eq!(client.device_uuid, [0x22; 16]);
            assert_eq!((client.max_bvlc, client.max_npdu), (8192, 4096));
            assert_eq!(client.heartbeat, heartbeat);
            assert_eq!(activity.load(Ordering::Acquire), 0);
            assert!(!closed.load(Ordering::Acquire));
        }
    }
    peer.ws
        .send(Message::Binary(vec![0x0a, 0, 0, 9].into()))
        .await
        .unwrap();
    assert_eq!(peer.binary().await, [0x0b, 0, 0, 9]);
    drop(sink);
    peer.close().await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn positive_limits_mtls_request_commits_exact_peer_capacities() {
    let tls = TestTls::new();
    let clients = clients();
    for (bvlc, npdu) in [
        (1u16, 1u16),
        (1, 65535),
        (65535, 1),
        (65535, 65535),
        (1200, 480),
        (300, 1476),
        (1476, 1476),
    ] {
        let mut peer = Peer::open(&tls, clients.clone()).await;
        let mut wire = valid_connect(6, [0x22; 6]);
        wire[26..28].copy_from_slice(&bvlc.to_be_bytes());
        wire[28..30].copy_from_slice(&npdu.to_be_bytes());
        peer.ws.send(Message::Binary(wire.into())).await.unwrap();
        assert_eq!(peer.binary().await[0..4], [7, 0, 0x22, 0x33]);
        assert!(peer.deadline.is_committed());
        {
            let map = clients.lock().await;
            assert_eq!(map.len(), 1);
            let client = map.get(&[0x22; 6]).unwrap();
            assert_eq!((client.max_bvlc, client.max_npdu), (bvlc, npdu));
            assert_eq!(client.device_uuid, [0x33; 16]);
        }
        peer.close().await;
        assert!(clients.lock().await.is_empty());
    }
}
