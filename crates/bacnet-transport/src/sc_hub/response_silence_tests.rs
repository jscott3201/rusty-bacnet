//! Accepting-hub response admission, not a universal BVLC response classifier.
use super::heartbeat_test_support::*;
use super::*;
use std::sync::atomic::AtomicU16;
use std::time::Duration;
use tokio::time::timeout;

// Independent AB.2.11/AB.2.13 wire vectors, without the production encoder.
pub(super) fn response(function: u8, id: u16) -> Vec<u8> {
    let mut wire = vec![function, 0];
    wire.extend_from_slice(&id.to_be_bytes());
    if function == 7 {
        wire.extend_from_slice(&[0x42; 6]);
        wire.extend_from_slice(&[0x22; 16]);
        wire.extend_from_slice(&[2, 0, 0, 128]);
    }
    wire
}

async fn send_raw(live: &mut LiveClient, wire: Vec<u8>) {
    live.ws.send(Message::Binary(wire.into())).await.unwrap();
}

async fn recv_raw(live: &mut LiveClient) -> Vec<u8> {
    match timeout(Duration::from_secs(2), live.ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        Message::Binary(data) => data.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

async fn barrier(live: &mut LiveClient) {
    // Invalid Request has a known NAK and does not refresh activity. TCP/WS
    // ordering proves every preceding response was dispatched without output.
    send_raw(live, vec![8, 0, 0x33, 0x44, 0x42]).await;
    assert_eq!(
        recv_raw(live).await,
        [0, 0, 0x33, 0x44, 8, 1, 0, 0, 7, 0, 7],
        "unsolicited response must not produce a reply before the barrier"
    );
}

async fn disconnect(live: &mut LiveClient) {
    send_raw(live, vec![8, 0, 0x55, 0x66]).await;
    assert_eq!(recv_raw(live).await, [9, 0, 0x55, 0x66]);
    live.expect_closed().await;
    timeout(Duration::from_secs(2), &mut live.reader)
        .await
        .unwrap()
        .unwrap();
}

async fn valid_response_is_silent(function: u8) {
    let clients = clients();
    let mut live = LiveClient::open(clients.clone(), [0x22; 6]).await;
    for registered in [false, true] {
        for id in [0, 1, 0x2233, u16::MAX] {
            send_raw(&mut live, response(function, id)).await;
            barrier(&mut live).await;
            assert_eq!(clients.lock().await.len(), usize::from(registered));
        }
        if !registered {
            live.ws
                .send(super::deadline_test_support::request([0x22; 6], [0x22; 16]))
                .await
                .unwrap();
            assert_eq!(live.recv().await.function, ScFunction::ConnectAccept);
        }
    }
    disconnect(&mut live).await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn unsolicited_connect_accept_is_silent_before_and_after_registration() {
    valid_response_is_silent(7).await;
}

#[tokio::test]
async fn unsolicited_disconnect_ack_is_silent_before_and_after_registration() {
    valid_response_is_silent(9).await;
}

fn response_cases(function: u8) -> Vec<Vec<u8>> {
    let valid = response(function, 0x2233);
    let payload = &valid[4..];
    let mut cases = vec![valid.clone()];
    for (flags, fields) in [
        (2, vec![0x9e, 0x1e]),             // optional MU-clear destination options
        (2, vec![0x5e]),                   // unsupported Must Understand
        (2, vec![0x9e, 0xfe, 0, 0, 0x5d]), // raw MU marker with empty data
        (8, vec![0x22; 6]),
        (8, vec![0; 6]),
        (4, vec![0x10; 6]),
        (4, vec![0x44; 6]),
        (4, vec![0xff; 6]),
        (1, vec![0x1e]), // forbidden Data Options
        (3, vec![0x5e, 0x1e]),
    ] {
        let mut wire = vec![function, flags, 0x22, 0x33];
        wire.extend(fields);
        wire.extend_from_slice(payload);
        assert!(decode_sc_message(&wire).is_ok());
        cases.push(wire);
    }
    let mut long = valid.clone();
    long.push(0x42);
    cases.push(long);
    if function == 7 {
        cases.push(vec![7, 0, 0x22, 0x33]);
        cases.push(valid[..29].to_vec());
        for field in [4..10, 10..26, 26..28, 28..30] {
            let mut wire = valid.clone();
            wire[field].fill(0);
            cases.push(wire);
        }
    }
    // Generic syntax errors still die in the existing decoder, not admission.
    for wire in [
        vec![function],
        vec![function, 0x80, 0x22, 0x33],
        vec![function, 8, 0x22, 0x33, 0x42],
        vec![function, 2, 0x22, 0x33, 0x9e],
        vec![function, 2, 0x22, 0x33, 0x3e, 0, 2, 0],
    ] {
        assert!(decode_sc_message(&wire).is_err());
        cases.push(wire);
    }
    cases
}

pub(super) struct Snapshot {
    sink: Arc<Mutex<WsSink>>,
    activity: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    notify: Arc<Notify>,
    uuid: DeviceUuid,
    limits: (u16, u16),
    heartbeat: heartbeat::HubHeartbeat,
    last_activity: u64,
}

impl Snapshot {
    pub(super) fn capture(client: &HubClient) -> Self {
        Self {
            sink: client.sink.clone(),
            activity: client.last_activity.clone(),
            closed: client.closed.clone(),
            notify: client.close_notify.clone(),
            uuid: client.device_uuid,
            limits: (client.max_bvlc, client.max_npdu),
            heartbeat: client.heartbeat,
            last_activity: client.last_activity.load(Ordering::Acquire),
        }
    }

    pub(super) fn unchanged(&self, client: &HubClient) {
        assert!(Arc::ptr_eq(&self.sink, &client.sink));
        assert!(Arc::ptr_eq(&self.activity, &client.last_activity));
        assert!(Arc::ptr_eq(&self.closed, &client.closed));
        assert!(Arc::ptr_eq(&self.notify, &client.close_notify));
        assert_eq!(self.uuid, client.device_uuid);
        assert_eq!(self.limits, (client.max_bvlc, client.max_npdu));
        assert_eq!(self.heartbeat, client.heartbeat);
        assert_eq!(
            self.last_activity,
            client.last_activity.load(Ordering::Acquire)
        );
        assert!(!client.closed.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn unsolicited_response_matrix_preserves_lease_activity_and_probe_then_recovers() {
    let clients = clients();
    let mut live = LiveClient::open(clients.clone(), [0x22; 6]).await;
    for registered in [false, true] {
        let before = if registered {
            live.idle().await;
            heartbeat::sweep(
                &clients,
                &AtomicU16::new(0x2233),
                &ClockIo(AtomicU64::new(100)),
            )
            .await;
            assert_eq!(recv_raw(&mut live).await, [0x0a, 0, 0x22, 0x33]);
            Some(Snapshot::capture(
                clients.lock().await.get(&live.vmac).unwrap(),
            ))
        } else {
            None
        };
        for function in [7, 9] {
            for wire in response_cases(function) {
                send_raw(&mut live, wire).await;
                barrier(&mut live).await;
                let map = clients.lock().await;
                assert_eq!(map.len(), usize::from(registered));
                if let Some(before) = &before {
                    before.unchanged(map.get(&live.vmac).unwrap());
                }
            }
        }
        if !registered {
            live.ws
                .send(super::deadline_test_support::request(live.vmac, [0x22; 16]))
                .await
                .unwrap();
            assert_eq!(live.recv().await.function, ScFunction::ConnectAccept);
        }
    }
    live.ack(0x2233).await;
    assert!(clients
        .lock()
        .await
        .get(&live.vmac)
        .unwrap()
        .heartbeat
        .pending
        .is_none());
    send_raw(&mut live, vec![0x0a, 0, 0x44, 0x55]).await;
    assert_eq!(recv_raw(&mut live).await, [0x0b, 0, 0x44, 0x55]);

    let mut recipient = LiveClient::connect(clients.clone(), [0x43; 6]).await;
    let mut npdu = vec![1, 4, 0x55, 0x66];
    npdu.extend_from_slice(&recipient.vmac);
    npdu.extend_from_slice(&[1, 0]);
    send_raw(&mut live, npdu).await;
    let mut expected = vec![1, 8, 0x55, 0x66];
    expected.extend_from_slice(&live.vmac);
    expected.extend_from_slice(&[1, 0]);
    assert_eq!(recv_raw(&mut recipient).await, expected);
    // A registered Encapsulated-NPDU Result is relayed, not discarded or NAKed.
    let mut result = vec![0, 4, 0x55, 0x66];
    result.extend_from_slice(&live.vmac);
    result.extend_from_slice(&[1, 1, 0, 0, 7, 0, 0x50]);
    send_raw(&mut recipient, result).await;
    let mut expected = vec![0, 8, 0x55, 0x66];
    expected.extend_from_slice(&recipient.vmac);
    expected.extend_from_slice(&[1, 1, 0, 0, 7, 0, 0x50]);
    assert_eq!(recv_raw(&mut live).await, expected);
    disconnect(&mut recipient).await;
    disconnect(&mut live).await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn unsolicited_responses_do_not_defer_idle_probe_or_its_original_timeout() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let ids = AtomicU16::new(0x2233);
    for now in [60, 100, 101, 102, 103, 104, 105, 106] {
        send_raw(&mut live, response(7, 0x2233)).await;
        send_raw(&mut live, response(9, 0x2233)).await;
        barrier(&mut live).await;
        assert_eq!(
            clients
                .lock()
                .await
                .get(&live.vmac)
                .unwrap()
                .last_activity
                .load(Ordering::Acquire),
            0
        );
        // Probe at t=100; keep the exact five-second boundary, retire at t=106.
        if now == 60 || now >= 100 {
            heartbeat::sweep(&clients, &ids, &ClockIo(AtomicU64::new(now))).await;
        }
        if now == 100 {
            assert_eq!(recv_raw(&mut live).await, [0x0a, 0, 0x22, 0x33]);
        }
        if (100..=105).contains(&now) {
            let map = clients.lock().await;
            assert_eq!(
                map.get(&live.vmac).unwrap().heartbeat.pending,
                Some(heartbeat::PendingHeartbeat {
                    message_id: 0x2233,
                    published_at: 100
                })
            );
        }
    }
    assert_eq!(ids.load(Ordering::Acquire), 0x2234, "no probe reseed");
    live.expect_closed().await;
    timeout(Duration::from_secs(2), &mut live.reader)
        .await
        .unwrap()
        .unwrap();
    assert!(clients.lock().await.is_empty());
    let mut recovery = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    disconnect(&mut recovery).await;
}
