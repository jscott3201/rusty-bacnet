//! Deterministic continuously-ready receive versus queued-send scheduling.
use super::super::direct_membership::{DirectMembership, DirectRole};
use super::*;
use bacnet_types::error::Error;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::sync::{oneshot, Semaphore};

struct ReadyPeer {
    incoming: Vec<u8>,
    reads: Arc<AtomicUsize>,
    // Capture how many ready incoming frames preceded each outgoing write.
    writes: Arc<Mutex<Vec<usize>>>,
}
impl WebSocketPort for ReadyPeer {
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        let n = self.reads.fetch_add(1, Ordering::SeqCst);
        if n < 32 {
            Ok(self.incoming.clone())
        } else {
            Err(Error::Encoding("end of ready fixture stream".into()))
        }
    }
    async fn send(&self, _: &[u8]) -> Result<(), Error> {
        self.writes
            .lock()
            .unwrap()
            .push(self.reads.load(Ordering::SeqCst));
        Ok(())
    }
}

#[tokio::test]
async fn direct_worker_ready_receive_stream_cannot_starve_queued_sends() {
    let owner = Arc::new(DirectMembership::default());
    let member = owner
        .reserve([2; 16], [2; 6], [1; 16], [1; 6], DirectRole::Outbound, 16)
        .unwrap()
        .commit();
    let reads = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let mut incoming = bytes::BytesMut::new();
    encode_sc_message(
        &mut incoming,
        &ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 1,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::from_static(&[1, 0, 0x30]),
        },
    );
    let permits = Arc::new(Semaphore::new(1));
    let (pooled, worker) = PooledDirect::start(
        DirectSocket::Custom(ReadyPeer {
            incoming: incoming.to_vec(),
            reads,
            writes: writes.clone(),
        }),
        member,
        permits.clone().try_acquire_owned().unwrap(),
        #[cfg(feature = "sc-tls")]
        None,
        (1476, 1476),
    );
    // Enqueue both sends synchronously before the spawned worker's first poll.
    // The peer's receive is also continuously ready. Its finite sentinel ends
    // a broken receive-only loop deterministically, without a sleep or timer.
    let (a, a_rx) = oneshot::channel();
    let (b, b_rx) = oneshot::channel();
    pooled
        .member
        .egress
        .send
        .try_send(DirectWrite {
            bytes: incoming.clone().freeze(),
            scopes: [None, None],
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
            started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            done: a,
        })
        .unwrap();
    pooled
        .member
        .egress
        .send
        .try_send(DirectWrite {
            bytes: incoming.clone().freeze(),
            scopes: [None, None],
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
            started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            done: b,
        })
        .unwrap();
    worker.await.unwrap();
    assert!(a_rx.await.unwrap().is_ok());
    assert!(b_rx.await.unwrap().is_ok());
    let observed = writes.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert!(
        observed[0] <= 1 && observed[1] <= 2,
        "ready reads before writes: {observed:?}"
    );
    assert!(!pooled.member.is_current());
    assert_eq!(permits.available_permits(), 1);
}
