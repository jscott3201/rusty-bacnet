//! Fair selection among the session's single-consumer inputs and worker joins.
use super::*;
use std::future::{poll_fn, Future};
use std::task::Poll;

pub(super) enum DispatchEvent {
    Worker(Result<(), tokio::task::JoinError>),
    Inbound(ReceivedApdu),
    Terminal(ReceivedApdu),
    Policy(PolicyOutcome),
    Closed,
}

pub(super) async fn next_event(parts: &mut DispatchParts, next: &mut usize) -> DispatchEvent {
    let notifications = &parts.notifications;
    let joined = async {
        match notifications {
            Some(owner) => owner.join_next().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(joined);
    let mut join_closed = false;
    poll_fn(|cx| {
        let mut receivers_closed = 0;
        // A continuously ready source gets one turn before the other ready
        // sources. This bounds both terminal delay and completed-task retention
        // to four selections, even when producers replenish on every turn.
        for offset in 0..4 {
            let index = (*next + offset) % 4;
            let event = match index {
                0 if !join_closed => match joined.as_mut().poll(cx) {
                    Poll::Ready(Some(result)) => Some(DispatchEvent::Worker(result)),
                    Poll::Ready(None) => {
                        join_closed = true;
                        None
                    }
                    Poll::Pending => None,
                },
                1 => match parts.inbound.poll_recv(cx) {
                    Poll::Ready(Some(received)) => Some(DispatchEvent::Inbound(received)),
                    Poll::Ready(None) => {
                        receivers_closed += 1;
                        None
                    }
                    Poll::Pending => None,
                },
                2 => match parts.terminal.poll_recv(cx) {
                    Poll::Ready(Some(received)) => Some(DispatchEvent::Terminal(received)),
                    Poll::Ready(None) => {
                        receivers_closed += 1;
                        None
                    }
                    Poll::Pending => None,
                },
                3 => match parts.policy.poll_recv(cx) {
                    Poll::Ready(Some(outcome)) => Some(DispatchEvent::Policy(outcome)),
                    Poll::Ready(None) => {
                        receivers_closed += 1;
                        None
                    }
                    Poll::Pending => None,
                },
                _ => None,
            };
            if let Some(event) = event {
                *next = (index + 1) % 4;
                return Poll::Ready(event);
            }
        }
        if receivers_closed == 3 {
            Poll::Ready(DispatchEvent::Closed)
        } else {
            Poll::Pending
        }
    })
    .await
}

#[cfg(test)]
#[path = "session_dispatch_tests.rs"]
mod tests;
