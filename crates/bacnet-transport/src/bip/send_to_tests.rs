//! The B/IP socket send's two paths (#1713): the try that almost always
//! sends at once, and the boxed wait when the socket would block.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::fanout::{AtomicFanoutCounters, FanoutJob, run_fanout_worker};
use super::socket::BipSocket;

async fn udp() -> UdpSocket {
    UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
}

fn address(socket: &UdpSocket) -> SocketAddrV4 {
    match socket.local_addr().unwrap() {
        std::net::SocketAddr::V4(address) => address,
        other => panic!("an IPv4 test socket, not {other}"),
    }
}

/// A send whose first try would block waits in the boxed send, and the
/// datagram still goes out once.
#[tokio::test]
async fn a_send_that_would_block_waits_and_sends_once() {
    let sender = BipSocket::new(udp().await, None);
    let receiver = udp().await;
    sender.block_next_try.store(true, Ordering::Relaxed);
    let sent = sender
        .send_to(b"blocked", address(&receiver))
        .await
        .unwrap();
    assert_eq!(sent, 7);
    assert!(!sender.block_next_try.load(Ordering::Relaxed));
    let mut buf = [0u8; 16];
    let (len, _) = receiver.recv_from(&mut buf).await.unwrap();
    assert_eq!(&buf[..len], b"blocked");
    sender.send_to(b"next", address(&receiver)).await.unwrap();
    let (len, _) = receiver.recv_from(&mut buf).await.unwrap();
    assert_eq!(&buf[..len], b"next", "one datagram per send");
}

/// The fanout worker, which only sends, still yields to the other tasks on
/// a current-thread runtime while it sends one job to 256 targets: each try
/// spends the task's cooperative budget as the awaited send did. This test
/// task sees the job part done only if the worker gave way mid-job.
#[tokio::test(flavor = "current_thread")]
async fn a_long_fanout_yields_to_other_tasks() {
    let socket = Arc::new(BipSocket::new(udp().await, None));
    let target = udp().await;
    let (tx, rx) = mpsc::channel(1);
    let counters = Arc::new(AtomicFanoutCounters::default());
    let worker = tokio::spawn(run_fanout_worker(socket, rx, Arc::clone(&counters)));
    tx.send(FanoutJob {
        frame: Bytes::from_static(&[0x81, 0x04, 0x00, 0x04]),
        targets: vec![address(&target); 256],
    })
    .await
    .unwrap();
    let mut part_done = None;
    loop {
        let forwarded = counters.packets_forwarded.load(Ordering::Relaxed);
        if forwarded == 256 {
            break;
        }
        if forwarded > 0 {
            part_done.get_or_insert(forwarded);
        }
        tokio::task::yield_now().await;
    }
    drop(tx);
    worker.await.unwrap();
    let part_done = part_done.expect("the worker sent all 256 without yielding");
    assert!(part_done < 256, "{part_done}");
}
