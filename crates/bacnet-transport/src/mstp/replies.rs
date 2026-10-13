//! Reply channels of the requests this station has passed up.
//!
//! The core holds one request at a time. While it does, the application's
//! answer through the request's channel goes to the core as the reply. Once
//! the core has finished the request without that answer (it sent Reply
//! Postponed at its decision deadline, or the requester may have stopped
//! waiting), the channel is kept, not closed: a deferred reply goes out
//! later, when this station holds the token (Clause 9.5.6.9), so an answer
//! that arrives then is queued as an ordinary NPDU to the requester.
//!
//! The deferred channels are bounded. At most [`DEFERRED_REPLIES`] are kept;
//! a new one past that drops the oldest, counted. Each expires at its
//! request's arrival plus the queue's maximum age: a channel is dropped
//! then, counted, and an answer queued before then keeps that same absolute
//! expiry in the queue, so no answer outlives its request by more than the
//! maximum age. The task sleeps no later than [`Replies::next_expiry`], so
//! the drop comes on time even when nothing else happens. A channel whose
//! sender is dropped just goes away.
//!
//! Each turn of the task's loop polls the current channel and every deferred
//! one ([`Replies::resolved`]): at most nine receivers, which costs little.
//! A much larger bound would want a ready queue instead, so that a turn
//! polls only the channels that have woken.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use bacnet_mstp_core::RequestId;
use bytes::Bytes;
use tokio::sync::oneshot;
use tokio::time::{Duration, Instant};

/// At most this many finished requests keep their reply channels.
pub(super) const DEFERRED_REPLIES: usize = 8;

/// The request the core still holds.
pub(super) struct Current {
    pub(super) id: RequestId,
    pub(super) requester: u8,
    /// The request's arrival plus the queue's maximum age.
    pub(super) expires: Instant,
    rx: oneshot::Receiver<Bytes>,
}

/// A finished request whose answer may still come.
struct Deferred {
    requester: u8,
    rx: oneshot::Receiver<Bytes>,
    expires: Instant,
}

/// What became of a request's channel when the core finished it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Finished {
    /// The answer was already there: queue it, to expire at `expires`.
    Answer {
        requester: u8,
        answer: Bytes,
        expires: Instant,
    },
    /// Kept for a late answer; `dropped_oldest` says the set was full.
    Deferred { dropped_oldest: bool },
    /// The sender is gone: nothing will come.
    Closed,
}

/// A late answer, or the end of a deferred channel.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Late {
    /// An answer in time: queue it to `requester`, to expire at `expires`.
    Answer {
        requester: u8,
        answer: Bytes,
        expires: Instant,
    },
    /// An answer, or nothing, after the channel's maximum age.
    Expired,
    /// The sender was dropped without an answer.
    Closed,
}

/// A channel that resolved.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Resolved {
    /// The current request's answer, or `None` if its sender was dropped;
    /// [`Replies::take_current`] says which request.
    Current(Option<Bytes>),
    /// A deferred channel.
    Late(Late),
}

/// The reply channels: the current request's and the deferred ones.
pub(super) struct Replies {
    current: Option<Current>,
    deferred: VecDeque<Deferred>,
    max_age: Duration,
}

impl Replies {
    pub(super) fn new(max_age: Duration) -> Self {
        Self {
            current: None,
            deferred: VecDeque::with_capacity(DEFERRED_REPLIES),
            max_age,
        }
    }

    /// Open the channel of a request that arrived at `now`; the caller has
    /// finished any older one first.
    pub(super) fn open(
        &mut self,
        id: RequestId,
        requester: u8,
        now: Instant,
    ) -> oneshot::Sender<Bytes> {
        debug_assert!(self.current.is_none(), "an older request is still open");
        let (tx, rx) = oneshot::channel();
        self.current = Some(Current {
            id,
            requester,
            expires: now + self.max_age,
            rx,
        });
        tx
    }

    /// The id of the request whose channel is current.
    pub(super) fn current_id(&self) -> Option<RequestId> {
        self.current.as_ref().map(|current| current.id)
    }

    /// Take the current request, whose answer has come.
    pub(super) fn take_current(&mut self) -> Option<Current> {
        self.current.take()
    }

    /// The core finished the current request without its answer.
    pub(super) fn finish_current(&mut self) -> Option<Finished> {
        let Current {
            requester,
            expires,
            mut rx,
            ..
        } = self.current.take()?;
        Some(match rx.try_recv() {
            Ok(answer) => Finished::Answer {
                requester,
                answer,
                expires,
            },
            Err(oneshot::error::TryRecvError::Closed) => Finished::Closed,
            Err(oneshot::error::TryRecvError::Empty) => {
                let dropped_oldest = self.deferred.len() == DEFERRED_REPLIES;
                if dropped_oldest {
                    self.deferred.pop_front();
                }
                self.deferred.push_back(Deferred {
                    requester,
                    rx,
                    expires,
                });
                Finished::Deferred { dropped_oldest }
            }
        })
    }

    /// How many deferred channels are kept.
    #[cfg(test)]
    pub(super) fn deferred(&self) -> usize {
        self.deferred.len()
    }

    /// When the first deferred channel expires, which the task wakes for.
    pub(super) fn next_expiry(&self) -> Option<Instant> {
        self.deferred.iter().map(|deferred| deferred.expires).min()
    }

    /// Drop the deferred channels that are past their age; return how many.
    pub(super) fn prune(&mut self, now: Instant) -> usize {
        let before = self.deferred.len();
        self.deferred.retain(|deferred| deferred.expires > now);
        before - self.deferred.len()
    }

    /// Whether any channel is open to wait on.
    pub(super) fn waiting(&self) -> bool {
        self.current.is_some() || !self.deferred.is_empty()
    }

    /// Wait for the current request's answer (or its sender to go), or for
    /// a deferred channel to resolve, which leaves the set. The current
    /// request comes first.
    pub(super) fn resolved(&mut self) -> impl Future<Output = Resolved> + '_ {
        std::future::poll_fn(move |cx| {
            if let Some(current) = self.current.as_mut() {
                if let Poll::Ready(result) = Pin::new(&mut current.rx).poll(cx) {
                    return Poll::Ready(Resolved::Current(result.ok()));
                }
            }
            let ready = self
                .deferred
                .iter_mut()
                .enumerate()
                .find_map(
                    |(index, deferred)| match Pin::new(&mut deferred.rx).poll(cx) {
                        Poll::Ready(result) => Some((index, result)),
                        Poll::Pending => None,
                    },
                );
            let Some((index, result)) = ready else {
                return Poll::Pending;
            };
            let deferred = self
                .deferred
                .remove(index)
                .expect("the index came from the set");
            Poll::Ready(Resolved::Late(match result {
                Ok(_) if Instant::now() >= deferred.expires => Late::Expired,
                Ok(answer) => Late::Answer {
                    requester: deferred.requester,
                    answer,
                    expires: deferred.expires,
                },
                Err(_) => Late::Closed,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGE: Duration = Duration::from_secs(5);

    fn request(replies: &mut Replies, id: u32) -> oneshot::Sender<Bytes> {
        replies.open(RequestId::new(id), 7, Instant::now())
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_already_given_is_returned_at_once() {
        let mut replies = Replies::new(AGE);
        let tx = request(&mut replies, 1);
        tx.send(Bytes::from_static(&[1])).unwrap();
        assert_eq!(
            replies.finish_current(),
            Some(Finished::Answer {
                requester: 7,
                answer: Bytes::from_static(&[1]),
                expires: Instant::now() + AGE,
            })
        );
        let tx = request(&mut replies, 2);
        drop(tx);
        assert_eq!(replies.finish_current(), Some(Finished::Closed));
        assert_eq!(replies.deferred(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_answer_comes_out_of_the_set() {
        let mut replies = Replies::new(AGE);
        let tx = request(&mut replies, 1);
        assert_eq!(
            replies.finish_current(),
            Some(Finished::Deferred {
                dropped_oldest: false
            })
        );
        let opened = Instant::now();
        tokio::time::advance(Duration::from_secs(1)).await;
        tx.send(Bytes::from_static(&[2])).unwrap();
        assert_eq!(
            replies.resolved().await,
            Resolved::Late(Late::Answer {
                requester: 7,
                answer: Bytes::from_static(&[2]),
                expires: opened + AGE,
            }),
            "the expiry counts from the request's arrival"
        );
        assert_eq!(replies.deferred(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_set_is_bounded_by_count_and_age() {
        let mut replies = Replies::new(AGE);
        let mut senders = Vec::new();
        for id in 0..=DEFERRED_REPLIES as u32 {
            senders.push(request(&mut replies, id));
            let dropped_oldest = id == DEFERRED_REPLIES as u32;
            assert_eq!(
                replies.finish_current(),
                Some(Finished::Deferred { dropped_oldest })
            );
        }
        assert_eq!(replies.deferred(), DEFERRED_REPLIES);
        assert!(senders[0].is_closed(), "the oldest was dropped");
        assert!(!senders[1].is_closed());

        tokio::time::advance(AGE).await;
        senders.pop().unwrap().send(Bytes::new()).unwrap();
        assert_eq!(replies.resolved().await, Resolved::Late(Late::Expired));
        assert_eq!(replies.prune(Instant::now()), DEFERRED_REPLIES - 1);
        assert_eq!(replies.deferred(), 0);
    }

    /// The first expiry is the oldest deferred channel's, and goes with it.
    #[tokio::test(start_paused = true)]
    async fn the_next_expiry_is_the_oldest_channels() {
        let mut replies = Replies::new(AGE);
        assert_eq!(replies.next_expiry(), None);
        let first = Instant::now();
        let _first = request(&mut replies, 1);
        assert_eq!(
            replies.next_expiry(),
            None,
            "the current one is not deferred"
        );
        replies.finish_current();
        tokio::time::advance(Duration::from_secs(1)).await;
        let _second = request(&mut replies, 2);
        replies.finish_current();
        assert_eq!(replies.next_expiry(), Some(first + AGE));
        assert_eq!(replies.prune(first + AGE), 1);
        assert_eq!(
            replies.next_expiry(),
            Some(first + Duration::from_secs(1) + AGE)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_sender_leaves_nothing_behind() {
        let mut replies = Replies::new(AGE);
        let tx = request(&mut replies, 1);
        replies.finish_current();
        drop(tx);
        assert_eq!(replies.resolved().await, Resolved::Late(Late::Closed));
        assert_eq!(replies.deferred(), 0);
    }
}
