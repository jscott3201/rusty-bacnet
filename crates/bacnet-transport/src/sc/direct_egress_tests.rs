//! Ordinary send disposition and shared immutable peer bounds.
use super::*;
use crate::port::{DirectResponse, DirectResponseScope, DirectScIdentity};
use crate::sc::direct_membership::{DirectMembership, DirectRole};
use std::{
    future::Future,
    task::{Context, Waker},
};
fn poll<F: Future>(f: std::pin::Pin<&mut F>) -> std::task::Poll<F::Output> {
    f.poll(&mut Context::from_waker(Waker::noop()))
}
fn member(limits: (u16, u16)) -> (Arc<DirectMembership>, Arc<Membership>) {
    let owner = Arc::new(DirectMembership::default());
    let member = owner
        .reserve([2; 16], [2; 6], [1; 16], [1; 6], DirectRole::Outbound, 1)
        .unwrap()
        .commit_with_limits(limits, Duration::from_secs(5));
    (owner, member)
}
#[tokio::test]
async fn ordinary_direct_unstarted_retirement_and_uncertain_started_write_are_distinct() {
    for started in [false, true] {
        let (_owner, member) = member((100, 96));
        let mut writes = member.take_writes();
        let send = member.egress.send_npdu(&[1, 0], &[]);
        tokio::pin!(send);
        assert!(poll(send.as_mut()).is_pending());
        let request = writes.recv().await.unwrap();
        if started {
            request.mark_started();
        }
        member.retire();
        drop(request);
        let result = send.await;
        if started {
            assert!(matches!(result, Err(DirectSendError::Failed(_))));
        } else {
            assert!(matches!(result, Err(DirectSendError::Unavailable)));
        }
    }
}
#[tokio::test]
async fn ordinary_and_original_response_share_sequence_queue_and_peer_limits() {
    let (_owner, member) = member((6, 2));
    let response = DirectResponse::new(
        &member,
        DirectScIdentity::verified([8; 32], member.generation),
    );
    let mut writes = member.take_writes();
    let mut first = Box::pin(member.egress.send_npdu(&[1, 0], &[]));
    assert!(poll(first.as_mut()).is_pending());
    let scope = DirectResponseScope::default();
    let mut second = Box::pin(response.send(&[1, 1], &scope));
    assert!(poll(second.as_mut()).is_pending());
    let a = writes.recv().await.unwrap();
    let b = writes.recv().await.unwrap();
    let a_id = crate::sc_frame::decode_sc_message(&a.bytes)
        .unwrap()
        .message_id;
    let b_id = crate::sc_frame::decode_sc_message(&b.bytes)
        .unwrap()
        .message_id;
    assert_eq!(b_id, a_id.wrapping_add(1));
    assert_eq!(a.bytes.len(), 6);
    assert_eq!(b.bytes.len(), 6);
    a.done.send(Ok(())).unwrap();
    b.done.send(Ok(())).unwrap();
    first.await.unwrap();
    second.await.unwrap();
    assert!(matches!(
        member.egress.send_npdu(&[1, 0, 3], &[]).await,
        Err(DirectSendError::Failed(Error::SendTooLong {
            length: 3,
            limit: 2
        }))
    ));
    let (_owner, small) = self::member((5, 2));
    assert!(matches!(
        small.egress.send_npdu(&[1, 0], &[]).await,
        Err(DirectSendError::Failed(Error::SendTooLong {
            length: 6,
            limit: 5
        }))
    ));
}
#[tokio::test(start_paused = true)]
async fn ordinary_direct_expired_queue_and_sealed_registry_never_reopen() {
    let (owner, member) = member((100, 96));
    let mut writes = member.take_writes();
    let mut send = Box::pin(member.egress.send_npdu(&[1, 0], &[]));
    assert!(poll(send.as_mut()).is_pending());
    let request = writes.recv().await.unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(!request.can_start());
    assert!(matches!(send.await, Err(DirectSendError::Failed(_))));
    let pending = owner
        .reserve([3; 16], [3; 6], [1; 16], [1; 6], DirectRole::Outbound, 2)
        .unwrap();
    owner.retire_all();
    owner.retire_all();
    let late = pending.commit_with_limits((100, 96), Duration::from_secs(5));
    assert!(!late.is_current());
    assert!(owner.route(&[3; 6]).is_none());
    assert!(
        owner
            .reserve([4; 16], [4; 6], [1; 16], [1; 6], DirectRole::Outbound, 2)
            .is_err()
    );
    assert!(matches!(
        member.egress.send_npdu(&[1, 0], &[]).await,
        Err(DirectSendError::Unavailable)
    ));
}
