use super::*;
use crate::sc::direct_membership::{DirectMembership, DirectRole};
use crate::sc_frame::decode_sc_message;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn route(
    limits: (u16, u16),
) -> (
    Arc<DirectMembership>,
    Arc<Membership>,
    DirectResponse,
    mpsc::Receiver<ResponseWrite>,
) {
    let registry = Arc::new(DirectMembership::default());
    let member = registry
        .reserve([1; 16], [1; 6], [2; 16], [2; 6], DirectRole::Accepted, 1)
        .unwrap()
        .commit_with_limits(limits, Duration::from_secs(5));
    let identity = DirectScIdentity::verified([3; 32], member.generation);
    let route = DirectResponse::new(&member, identity);
    let recv = member.take_writes();
    (registry, member, route, recv)
}

#[tokio::test]
async fn direct_response_queue_bound_cancellation_and_weak_membership() {
    let scope = DirectResponseScope::default();
    let (registry, member, route, mut recv) = route((100, 96));
    let mut pending: Vec<_> = (0..RESPONSE_QUEUE_CAPACITY)
        .map(|_| Box::pin(route.send(&[1, 0], &scope)))
        .collect();
    for future in &mut pending {
        assert!(poll_once(future.as_mut()).is_pending());
    }
    assert_eq!(recv.len(), RESPONSE_QUEUE_CAPACITY);
    assert!(matches!(
        poll_once(Box::pin(route.send(&[1, 0], &scope)).as_mut()),
        Poll::Ready(Err(_))
    ));
    drop(pending);
    let first = recv.recv().await.unwrap();
    assert!(
        first.done.is_closed(),
        "cancelled queued work is detectable before write"
    );
    drop(member);
    assert!(
        registry.current_generations().is_empty(),
        "retained routes cannot keep membership alive"
    );
    assert!(route.send(&[1, 0], &scope).await.is_err());
}

#[tokio::test]
async fn direct_response_negotiated_npdu_and_complete_bvlc_limits() {
    let scope = DirectResponseScope::default();
    let (_, _member, route, mut recv) = route((6, 2));
    assert!(matches!(
        route.send(&[1, 0, 0], &scope).await,
        Err(Error::Encoding(_))
    ));
    assert!(recv.try_recv().is_err());
    let mut send = Box::pin(route.send(&[1, 0], &scope));
    assert!(poll_once(send.as_mut()).is_pending());
    let write = recv.recv().await.unwrap();
    assert_eq!(write.bytes.len(), 6);
    let frame = decode_sc_message(&write.bytes).unwrap();
    assert_eq!(frame.originating_vmac, None);
    assert_eq!(frame.destination_vmac, None);
    assert!(frame.data_options.is_empty());
    write.done.send(Ok(())).unwrap();
    send.await.unwrap();
    let (_registry, _small_member, small, mut receiver) = self::route((5, 2));
    assert!(matches!(
        small.send(&[1, 0], &scope).await,
        Err(Error::Encoding(_))
    ));
    assert!(receiver.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn direct_response_queue_wait_deadline_and_retirement_are_bounded() {
    let scope = DirectResponseScope::default();
    let (_, member, route, mut recv) = route((100, 96));
    let send = route.send(&[1, 0], &scope);
    tokio::pin!(send);
    assert!(poll_once(send.as_mut()).is_pending());
    let write = recv.recv().await.unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(
        !write.can_start(),
        "expired queue work is skipped even before its waiter polls timeout"
    );
    assert!(send.await.is_err());
    assert!(write.done.is_closed());
    member.retire();
    assert!(route.send(&[1, 0], &scope).await.is_err());
    assert!(recv.try_recv().is_err());
}

#[tokio::test]
async fn direct_response_owner_seal_and_drop_cancel_queued_work_irreversibly() {
    let (_, _member, route, mut recv) = route((100, 96));
    let scope = DirectResponseScope::default();
    let mut send = Box::pin(route.send(&[1, 0], &scope));
    assert!(poll_once(send.as_mut()).is_pending());
    let write = recv.recv().await.unwrap();
    assert!(write.can_start());
    scope.seal();
    scope.seal();
    assert!(
        !write.can_start(),
        "seal rejects work whose waiter is still live"
    );
    assert!(route.send(&[1, 0], &scope).await.is_err());
    drop(write);
    assert!(send.await.is_err());
    let next_owner = DirectResponseScope::default();
    let mut send = Box::pin(route.send(&[1, 0], &next_owner));
    assert!(poll_once(send.as_mut()).is_pending());
    let write = recv.recv().await.unwrap();
    assert!(
        write.can_start(),
        "a fresh owner may use a still-live socket"
    );
    drop(send);
    drop(next_owner);
    assert!(!write.can_start());
    assert!(
        route.send(&[1, 0], &scope).await.is_err(),
        "old owner never reopens"
    );
}

#[tokio::test]
async fn direct_response_both_owner_guards_retire_unstarted_writes() {
    for seal_router in [false, true] {
        let (_, _member, response, mut recv) = route((100, 96));
        let network = DirectResponseScope::default();
        let router = DirectResponseScope::default();
        let guard = router.guard();
        let mut send = Box::pin(response.send_with_owner(&[1, 0], &network, &guard));
        assert!(poll_once(send.as_mut()).is_pending());
        let write = recv.recv().await.unwrap();
        assert!(write.can_start());
        if seal_router {
            router.seal();
        } else {
            network.seal();
        }
        assert!(
            !write.can_start(),
            "either owner retires a still-waiting write"
        );
        drop(write);
        assert!(send.await.is_err());
        assert!(
            response
                .send_with_owner(&[1, 0], &network, &guard)
                .await
                .is_err()
        );
        assert!(recv.try_recv().is_err());
    }
    let scope = DirectResponseScope::default();
    let guard = scope.guard();
    drop(scope);
    assert!(
        !guard.is_open(),
        "a retained guard cannot keep its owner alive"
    );
}
