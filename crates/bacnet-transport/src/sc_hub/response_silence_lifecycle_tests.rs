//! Real authenticated WebSockets with controlled accepting-peer deadlines.
use super::deadline_test_support::*;
use super::heartbeat_test_support::clients;
use super::response_silence_tests::{response, Snapshot};
use super::*;
use std::time::Duration;

async fn barrier(peer: &mut DeadlinePeer) {
    peer.ws
        .send(Message::Binary(vec![8, 0, 0x33, 0x44, 0x42].into()))
        .await
        .unwrap();
    assert_eq!(
        peer.next().await,
        Message::Binary(vec![0, 0, 0x33, 0x44, 8, 1, 0, 0, 7, 0, 7].into())
    );
}

async fn close(peer: &mut DeadlinePeer) {
    peer.ws.close(None).await.unwrap();
    poll_io(&mut peer.task).await.unwrap();
    assert_eq!(peer.active.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn unsolicited_responses_mtls_keep_absolute_connect_deadline_and_release_admission() {
    let clients = clients();
    let mut peer = DeadlinePeer::new(clients.clone(), Duration::from_secs(1)).await;
    tokio::time::pause();
    let expires = peer.deadline.expires();
    for id in 0..9 {
        tokio::time::advance(Duration::from_millis(100)).await;
        for function in [7, 9] {
            peer.ws
                .send(Message::Binary(response(function, id).into()))
                .await
                .unwrap();
        }
        barrier(&mut peer).await;
        assert!(!peer.deadline.admission_started.load(Ordering::Acquire));
        assert!(!peer.deadline.is_committed());
        assert_eq!(peer.deadline.expires(), expires);
        assert_eq!(peer.active.load(Ordering::Acquire), 1);
        assert!(clients.lock().await.is_empty());
    }
    tokio::time::advance(Duration::from_millis(102)).await;
    poll_io(&mut peer.task).await.unwrap();
    assert!(matches!(peer.next().await, Message::Close(_)));
    assert_eq!(peer.active.load(Ordering::Acquire), 0);
    assert!(!peer.deadline.is_committed());
    assert!(clients.lock().await.is_empty());
    tokio::time::resume();
    let mut recovery = DeadlinePeer::new(clients.clone(), Duration::from_secs(5)).await;
    recovery
        .ws
        .send(request([0x42; 6], [0x22; 16]))
        .await
        .unwrap();
    assert!(matches!(recovery.next().await, Message::Binary(data) if data[0] == 7));
    close(&mut recovery).await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn unsolicited_responses_at_capacity_preserve_owners_then_allow_real_replacement() {
    let clients = clients();
    let mut peers = Vec::new();
    for index in 1u16..=256 {
        let mut vmac = [2, 0, 0, 0, 0, 0];
        vmac[4..].copy_from_slice(&index.to_be_bytes());
        let mut uuid = [0; 16];
        uuid[14..].copy_from_slice(&index.to_be_bytes());
        let mut peer = DeadlinePeer::new(clients.clone(), Duration::from_secs(5)).await;
        peer.ws.send(request(vmac, uuid)).await.unwrap();
        assert!(matches!(peer.next().await, Message::Binary(data) if data[0] == 7));
        peers.push(peer);
    }
    let before: Vec<_> = clients
        .lock()
        .await
        .iter()
        .map(|(vmac, client)| (*vmac, Snapshot::capture(client)))
        .collect();
    let mut candidate = DeadlinePeer::new(clients.clone(), Duration::from_secs(5)).await;
    for proposed_vmac in [[2, 0, 0, 0, 0, 1], [0x42; 6], [0x10; 6]] {
        let mut accept = response(7, 0x2233);
        accept[4..10].copy_from_slice(&proposed_vmac);
        accept[10..26].fill(0);
        accept[25] = 1; // incumbent UUID, same/different/colliding hub VMAC
        candidate
            .ws
            .send(Message::Binary(accept.into()))
            .await
            .unwrap();
        candidate
            .ws
            .send(Message::Binary(response(9, 0x2233).into()))
            .await
            .unwrap();
        barrier(&mut candidate).await;
        assert!(!candidate.deadline.admission_started.load(Ordering::Acquire));
        assert!(!candidate.deadline.is_committed());
        let map = clients.lock().await;
        assert_eq!(map.len(), 256);
        for (vmac, snapshot) in &before {
            snapshot.unchanged(map.get(vmac).unwrap());
        }
    }
    // The same candidate can still make a real Request and replace at capacity.
    let mut uuid = [0; 16];
    uuid[15] = 1;
    candidate.ws.send(request([0x42; 6], uuid)).await.unwrap();
    assert!(matches!(candidate.next().await, Message::Binary(data) if data[0] == 7));
    assert!(candidate.deadline.is_committed());
    poll_io(&mut peers[0].task).await.unwrap();
    assert!(matches!(peers[0].next().await, Message::Close(_)));
    assert_eq!(peers[0].active.load(Ordering::Acquire), 0);
    assert_eq!(clients.lock().await.len(), 256);
    assert!(!clients.lock().await.contains_key(&[2, 0, 0, 0, 0, 1]));
    for (vmac, snapshot) in &before {
        if *vmac != [2, 0, 0, 0, 0, 1] {
            snapshot.unchanged(clients.lock().await.get(vmac).unwrap());
        }
    }
    let mut full = DeadlinePeer::new(clients.clone(), Duration::from_secs(5)).await;
    full.ws.send(request([0x43; 6], [0x43; 16])).await.unwrap();
    assert_eq!(
        full.next().await,
        Message::Binary(vec![0, 0, 0x22, 0x33, 6, 1, 0, 0, 3, 0, 0].into())
    );
    poll_io(&mut full.task).await.unwrap();
    assert_eq!(full.active.load(Ordering::Acquire), 0);
    drop(before);
    close(&mut candidate).await;
    for peer in peers.iter_mut().skip(1) {
        close(peer).await;
    }
    assert!(clients.lock().await.is_empty());
}
