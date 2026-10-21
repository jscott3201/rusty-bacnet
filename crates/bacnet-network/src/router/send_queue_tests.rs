use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::task::{Context, Poll, Wake, Waker};

use bacnet_npdu::NetworkPriority;
use bytes::Bytes;

use super::{channel, SendRequest, TryRecvError, TrySendError, CAPACITY};

fn request(priority: u8, tag: u8) -> SendRequest {
    SendRequest::Broadcast {
        npdu: Bytes::from(vec![priority, tag]),
        data_attributes: Vec::new(),
        reject_origin: None,
    }
}

/// The lane for a priority given as its value, 0 to 3.
fn prio(value: u8) -> NetworkPriority {
    NetworkPriority::try_from(value).unwrap()
}

fn tag(request: SendRequest) -> (u8, u8) {
    let SendRequest::Broadcast { npdu, .. } = request else {
        panic!("expected broadcast");
    };
    (npdu[0], npdu[1])
}

#[test]
fn all_priorities_share_capacity_without_eviction_and_dequeue_releases_a_slot() {
    let (tx, mut rx) = channel(CAPACITY);
    for index in 0..CAPACITY {
        let priority = (index % 4) as u8;
        tx.try_send(prio(priority), request(priority, index as u8))
            .unwrap();
    }
    let Err(TrySendError::Full(())) = tx.try_send(prio(3), request(3, 99)) else {
        panic!("a full queue must refuse even life safety");
    };
    assert_eq!(tag(rx.try_recv().unwrap()), (3, 3));
    tx.try_send(prio(3), request(3, 99)).unwrap();
    let mut expected = Vec::new();
    for priority in (0..4).rev() {
        expected.extend((0..CAPACITY).filter_map(|index| {
            ((index % 4) as u8 == priority && index != 3).then_some((priority, index as u8))
        }));
        if priority == 3 {
            expected.push((3, 99));
        }
    }
    let mut sent = Vec::new();
    while let Ok(item) = rx.try_recv() {
        sent.push(tag(item));
    }
    assert_eq!(sent, expected);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn concurrent_senders_cannot_over_admit_or_reorder_their_lane() {
    let (tx, mut rx) = channel(64);
    let start = Arc::new(Barrier::new(5));
    let workers: Vec<_> = (0..4)
        .map(|priority| {
            let tx = tx.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                let mut admitted = 0;
                for index in 0..100 {
                    match tx.try_send(prio(priority), request(priority, index)) {
                        Ok(()) => admitted += 1,
                        Err(TrySendError::Full(_)) => {}
                        Err(TrySendError::Closed(_)) => panic!("receiver is still alive"),
                    }
                }
                admitted
            })
        })
        .collect();
    start.wait();
    let admitted: usize = workers.into_iter().map(|w| w.join().unwrap()).sum();
    assert_eq!(admitted, 64);
    drop(tx);
    let mut sent = Vec::new();
    while let Ok(item) = rx.try_recv() {
        sent.push(tag(item));
    }
    assert_eq!(sent.len(), admitted);
    assert!(sent.windows(2).all(|pair| {
        pair[0].0 > pair[1].0 || (pair[0].0 == pair[1].0 && pair[0].1 < pair[1].1)
    }));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
}

struct Owned {
    _lifetime: Arc<()>,
    data: [u8; 2],
}

impl AsRef<[u8]> for Owned {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

#[test]
fn receiver_drop_releases_every_lane_and_closed_wins_over_full() {
    let (tx, rx) = channel(4);
    let mut lifetimes = Vec::new();
    for priority in 0..4 {
        let lifetime = Arc::new(());
        lifetimes.push(Arc::downgrade(&lifetime));
        let npdu = Bytes::from_owner(Owned {
            _lifetime: lifetime,
            data: [priority, 1],
        });
        tx.try_send(
            prio(priority),
            SendRequest::Broadcast {
                npdu,
                data_attributes: Vec::new(),
                reject_origin: None,
            },
        )
        .unwrap();
    }
    assert!(lifetimes.iter().all(|weak| weak.upgrade().is_some()));
    drop(rx);
    assert!(lifetimes.iter().all(|weak| weak.upgrade().is_none()));
    let Err(TrySendError::Closed(())) = tx.try_send(prio(3), request(3, 88)) else {
        panic!("dropped receiver must close admission");
    };
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn receive_wakes_on_send_and_last_sender_drop_and_survives_cancellation() {
    let (tx, mut rx) = channel(4);
    let wake = Arc::new(WakeCount::default());
    let waker = Waker::from(Arc::clone(&wake));
    let mut context = Context::from_waker(&waker);
    let mut receive = Box::pin(rx.recv());
    assert!(receive.as_mut().poll(&mut context).is_pending());
    tx.try_send(prio(0), request(0, 1)).unwrap();
    assert!(wake.0.load(Ordering::SeqCst) > 0);
    // Cancel after notification but before consuming the queued send.
    drop(receive);
    tx.try_send(prio(3), request(3, 2)).unwrap();
    let mut receive = Box::pin(rx.recv());
    let Poll::Ready(Some(item)) = receive.as_mut().poll(&mut context) else {
        panic!("queued high priority must be ready after cancellation");
    };
    assert_eq!(tag(item), (3, 2));
    drop(receive);
    assert_eq!(tag(rx.try_recv().unwrap()), (0, 1));

    let last = tx.clone();
    let mut receive = Box::pin(rx.recv());
    assert!(receive.as_mut().poll(&mut context).is_pending());
    wake.0.store(0, Ordering::SeqCst);
    drop(tx);
    assert!(receive.as_mut().poll(&mut context).is_pending());
    drop(last);
    assert!(wake.0.load(Ordering::SeqCst) > 0);
    assert!(matches!(
        receive.as_mut().poll(&mut context),
        Poll::Ready(None)
    ));
}
