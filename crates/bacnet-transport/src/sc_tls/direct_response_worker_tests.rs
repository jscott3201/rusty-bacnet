//! Deterministic scheduling and blocked-writer controls on the production loop.
use super::*;
use crate::port::{DirectResponse, DirectResponseScope, DirectScIdentity};
use std::sync::atomic::AtomicUsize;
use tokio::sync::oneshot;

enum Read {
    Binary,
    Pending,
    Controls(Arc<AtomicUsize>),
    /// These reads, then the end of the stream.
    Frames(std::collections::VecDeque<Result<DirectFrame, String>>),
}
impl DirectWsRead for Read {
    async fn next_frame(&mut self) -> Option<Result<DirectFrame, String>> {
        match self {
            Self::Binary => Some(Ok(DirectFrame::Binary(vec![0xff]))),
            Self::Frames(frames) => frames.pop_front(),
            Self::Pending => std::future::pending().await,
            Self::Controls(count) => {
                count.fetch_add(1, Ordering::AcqRel);
                Some(Ok(DirectFrame::Control))
            }
        }
    }
}
struct Write {
    writes: Arc<AtomicUsize>,
    entered: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
}
impl DirectWs for Write {
    type Read = Read;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()> {
        if decode_sc_message(data).unwrap().function == ScFunction::EncapsulatedNpdu {
            self.writes.fetch_add(1, Ordering::AcqRel);
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
            }
            if let Some(release) = self.release.take() {
                release.await.map_err(|_| ())?;
            }
        }
        Ok(())
    }
    async fn send_close(&mut self) -> Result<(), ()> {
        Ok(())
    }
}
fn route() -> (
    Arc<Membership>,
    DirectResponse,
    mpsc::Receiver<crate::direct_response::ResponseWrite>,
) {
    let registry = Arc::new(DirectMembership::default());
    let member = registry
        .reserve([1; 16], [1; 6], [2; 16], [2; 6], DirectRole::Accepted, 1)
        .unwrap()
        .commit_with_limits((100, 96), Duration::from_secs(10));
    let identity = DirectScIdentity::verified([3; 32], member.generation);
    let route = DirectResponse::new(&member, identity);
    let recv = member.take_writes();
    (member, route, recv)
}
#[tokio::test]
async fn direct_response_ready_read_and_queue_have_bounded_alternating_progress() {
    let (_member, route, mut recv) = route();
    let scope = DirectResponseScope::default();
    let mut send = Box::pin(route.send(&[1, 0], &scope));
    assert!(futures_util::poll!(&mut send).is_pending());
    let mut read = Read::Binary;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    assert!(matches!(
        next_event(&mut read, &mut recv, false, deadline).await,
        Event::Read(_)
    ));
    let Event::Write(Some(write)) = next_event(&mut read, &mut recv, true, deadline).await else {
        panic!("ready read starved queued send")
    };
    write.done.send(Ok(())).unwrap();
    send.await.unwrap();
    let mut send = Box::pin(route.send(&[1, 0], &scope));
    assert!(futures_util::poll!(&mut send).is_pending());
    assert!(
        matches!(
            next_event(&mut read, &mut recv, false, deadline).await,
            Event::Read(_)
        ),
        "ready queue must not starve control"
    );
}

async fn worker(
    member: Arc<Membership>,
    route: DirectResponse,
    recv: mpsc::Receiver<crate::direct_response::ResponseWrite>,
    write: Write,
    read: Read,
) {
    let warnings = DirectWarnings::default();
    worker_warning_to(member, route, recv, write, read, &warnings).await;
}

/// [`worker`], warning through `warnings`.
async fn worker_warning_to(
    member: Arc<Membership>,
    route: DirectResponse,
    mut recv: mpsc::Receiver<crate::direct_response::ResponseWrite>,
    mut write: Write,
    mut read: Read,
    warnings: &DirectWarnings,
) {
    let ca = super::super::direct_accept_tests::TestCa::generate();
    let config = DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        [2; 6],
        [2; 16],
        ca.node_config(vec!["localhost".into()]),
    )
    .with_connect_timeout(Duration::from_secs(5))
    .with_idle_timeout(Duration::from_secs(10));
    let _retire = RetireMember(&member);
    let (tx, _rx) = mpsc::channel(4);
    let admission = Arc::new(ScNpduAdmission::new(ScNpduAdmissionPolicy::default()));
    serve_npdu_loop(
        &mut write,
        &mut read,
        &config,
        AdmittedDirectPeer {
            address: "127.0.0.1:1".parse().unwrap(),
            member: &member,
            identity: route.identity(),
            response: route,
            warnings,
        },
        &tx,
        &admission,
        &mut recv,
    )
    .await;
}

#[tokio::test]
async fn direct_response_started_write_may_finish_but_sealed_queued_write_cannot_start() {
    let (member, route, recv) = route();
    let scope = DirectResponseScope::default();
    let (entered, barrier) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let writes = Arc::new(AtomicUsize::new(0));
    let worker = tokio::spawn(worker(
        member.clone(),
        route.clone(),
        recv,
        Write {
            writes: writes.clone(),
            entered: Some(entered),
            release: Some(gate),
        },
        Read::Pending,
    ));
    let mut first = Box::pin(route.send(&[1, 0], &scope));
    assert!(futures_util::poll!(&mut first).is_pending());
    barrier.await.unwrap();
    let mut second = Box::pin(route.send(&[1, 1], &scope));
    assert!(futures_util::poll!(&mut second).is_pending());
    scope.seal();
    release.send(()).unwrap();
    first.await.unwrap();
    assert!(second.await.is_err());
    assert_eq!(writes.load(Ordering::Acquire), 1);
    member.retire();
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn direct_response_blocked_write_deadline_retires_socket_and_drops_queued_work() {
    let (member, route, recv) = route();
    let scope = DirectResponseScope::default();
    let (entered, barrier) = oneshot::channel();
    let (_release, gate) = oneshot::channel();
    let writes = Arc::new(AtomicUsize::new(0));
    let worker = tokio::spawn(worker(
        member.clone(),
        route.clone(),
        recv,
        Write {
            writes: writes.clone(),
            entered: Some(entered),
            release: Some(gate),
        },
        Read::Pending,
    ));
    let mut first = Box::pin(route.send(&[1, 0], &scope));
    assert!(futures_util::poll!(&mut first).is_pending());
    barrier.await.unwrap();
    let mut second = Box::pin(route.send(&[1, 1], &scope));
    assert!(futures_util::poll!(&mut second).is_pending());
    tokio::time::advance(Duration::from_secs(5)).await;
    worker.await.unwrap();
    assert!(first.await.is_err());
    assert!(second.await.is_err());
    assert!(!member.is_current());
    assert_eq!(writes.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn direct_response_ready_websocket_controls_return_a_scheduling_turn() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let (_member, route, mut recv) = route();
    let scope = DirectResponseScope::default();
    let mut send = Box::pin(route.send(&[1, 0], &scope));
    let consumed = Arc::new(AtomicUsize::new(0));
    let count = consumed.clone();
    let controls = (0..8).map(|i| {
        Ok::<_, String>(if i % 2 == 0 {
            Message::Ping(vec![i].into())
        } else {
            Message::Pong(vec![i].into())
        })
    });
    let frames = controls.chain([Ok(Message::Binary(vec![0xff].into()))]);
    let mut read = futures_util::stream::iter(frames).inspect(|_| {
        if count.fetch_add(1, Ordering::AcqRel) == 0 {
            // Enqueue while the adapter is already polling its ready input.
            assert!(std::future::Future::poll(
                send.as_mut(),
                &mut std::task::Context::from_waker(std::task::Waker::noop()),
            )
            .is_pending());
        }
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    assert!(matches!(
        next_event(&mut read, &mut recv, false, deadline).await,
        Event::Read(Ok(Some(Ok(_))))
    ));
    assert_eq!(consumed.load(Ordering::Acquire), 1, "one control per turn");
    let Event::Write(Some(write)) = next_event(&mut read, &mut recv, true, deadline).await else {
        panic!("ready controls starved a response queued during input polling")
    };
    assert_eq!(consumed.load(Ordering::Acquire), 1);
    write.done.send(Ok(())).unwrap();
    drop(read);
    send.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn direct_response_websocket_controls_preserve_connect_and_idle_deadlines() {
    use tokio_tungstenite::tungstenite::Message;
    let mut ready = futures_util::stream::repeat(Ok::<_, String>(Message::Ping(vec![1].into())));
    let handshake = tokio::time::timeout(Duration::from_secs(5), ready.next_data());
    tokio::pin!(handshake);
    assert!(futures_util::poll!(&mut handshake).is_pending());
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(
        handshake.await.is_err(),
        "controls cannot extend Connect deadline"
    );

    let (member, route, recv) = route();
    let consumed = Arc::new(AtomicUsize::new(0));
    let worker = tokio::spawn(worker(
        member.clone(),
        route,
        recv,
        Write {
            writes: Arc::new(AtomicUsize::new(0)),
            entered: None,
            release: None,
        },
        Read::Controls(consumed.clone()),
    ));
    while consumed.load(Ordering::Acquire) == 0 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(10)).await;
    worker.await.unwrap();
    assert!(
        !member.is_current(),
        "controls cannot extend binary-activity idle deadline"
    );
}

/// #1950: a burst of bad frames on an established connection, undecodable
/// and oversized, then a failed read, warns once through the listener's
/// frame throttle; the rest log at debug and are counted for the next one.
#[tokio::test(start_paused = true)]
async fn a_burst_of_bad_frames_warns_once_per_interval() {
    use crate::warn_throttle::WARNING_INTERVAL;
    let (member, route, recv) = route();
    let oversized = vec![0; usize::from(crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH) + 1];
    let mut frames: std::collections::VecDeque<_> =
        std::iter::repeat_with(|| Ok(DirectFrame::Binary(vec![0xff])))
            .take(3)
            .collect();
    frames.extend([
        Ok(DirectFrame::Binary(oversized.clone())),
        Ok(DirectFrame::Binary(oversized)),
        Err("connection reset".to_string()),
    ]);
    let warnings = DirectWarnings::default();
    let write = Write {
        writes: Arc::new(AtomicUsize::new(0)),
        entered: None,
        release: None,
    };
    worker_warning_to(member, route, recv, write, Read::Frames(frames), &warnings).await;
    assert_eq!(
        warnings.frame.note(),
        None,
        "the first bad frame warned; the rest, and this, are held back"
    );
    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(
        warnings.frame.note(),
        Some(7),
        "five held back, the check above, and this one"
    );
    assert_eq!(
        warnings.connection.note(),
        Some(1),
        "an established connection's frames don't touch the connection throttle"
    );
}
