//! Priority-aware channel for BACnet NPDU dispatch (Clause 6.2.2).
//!
//! BACnet defines four priority levels: Normal, Urgent, Critical Equipment,
//! and Life Safety. Higher-priority messages must be dispatched before
//! lower-priority ones. This module provides a multi-queue channel where
//! the receiver always drains the highest-priority queue first.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Notify;

use bacnet_types::enums::NetworkPriority;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// An item tagged with a BACnet network priority.
#[derive(Debug, Clone)]
pub struct PrioritizedItem<T> {
    pub priority: NetworkPriority,
    pub data: T,
}

/// Cloneable sender half of a priority channel.
pub struct PrioritySender<T> {
    queues: Arc<Mutex<[VecDeque<PrioritizedItem<T>>; 4]>>,
    notify: Arc<Notify>,
    capacity: usize,
    /// Shared token — receiver holds a `Weak` to detect all senders dropped.
    _token: Arc<()>,
}

impl<T> Clone for PrioritySender<T> {
    fn clone(&self) -> Self {
        Self {
            queues: Arc::clone(&self.queues),
            notify: Arc::clone(&self.notify),
            capacity: self.capacity,
            _token: Arc::clone(&self._token),
        }
    }
}

impl<T> Drop for PrioritySender<T> {
    fn drop(&mut self) {
        // Wake the receiver so it can check the closed condition.
        self.notify.notify_one();
    }
}

/// Receiver half of a priority channel (not cloneable).
pub struct PriorityReceiver<T> {
    queues: Arc<Mutex<[VecDeque<PrioritizedItem<T>>; 4]>>,
    notify: Arc<Notify>,
    sender_token: Weak<()>,
}

// ---------------------------------------------------------------------------
// Priority index mapping
// ---------------------------------------------------------------------------

/// Map a `NetworkPriority` to a queue index (0 = highest priority).
pub fn priority_index(p: NetworkPriority) -> usize {
    if p == NetworkPriority::LIFE_SAFETY {
        0
    } else if p == NetworkPriority::CRITICAL_EQUIPMENT {
        1
    } else if p == NetworkPriority::URGENT {
        2
    } else {
        3 // NORMAL and any unknown/vendor values
    }
}

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

/// Create a priority channel with `capacity` slots per priority level.
///
/// Returns a `(PrioritySender, PriorityReceiver)` pair.
pub fn priority_channel<T>(capacity: usize) -> (PrioritySender<T>, PriorityReceiver<T>) {
    let queues = Arc::new(Mutex::new([
        VecDeque::with_capacity(capacity),
        VecDeque::with_capacity(capacity),
        VecDeque::with_capacity(capacity),
        VecDeque::with_capacity(capacity),
    ]));
    let notify = Arc::new(Notify::new());
    let token = Arc::new(());
    let weak = Arc::downgrade(&token);

    let tx = PrioritySender {
        queues: Arc::clone(&queues),
        notify: Arc::clone(&notify),
        capacity,
        _token: token,
    };

    let rx = PriorityReceiver {
        queues,
        notify,
        sender_token: weak,
    };

    (tx, rx)
}

// ---------------------------------------------------------------------------
// Sender
// ---------------------------------------------------------------------------

impl<T> PrioritySender<T> {
    /// Enqueue an item into the appropriate priority queue.
    ///
    /// Returns `Err(item)` if that priority's queue is at capacity.
    /// The method is async for API consistency but does not block.
    pub async fn send(&self, item: PrioritizedItem<T>) -> Result<(), PrioritizedItem<T>> {
        {
            let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
            let idx = priority_index(item.priority);
            let q = &mut queues[idx];
            if q.len() >= self.capacity {
                return Err(item);
            }
            q.push_back(item);
        }
        self.notify.notify_one();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Receiver
// ---------------------------------------------------------------------------

impl<T> PriorityReceiver<T> {
    /// Receive the next item, highest priority first.
    ///
    /// Returns `None` when all senders have been dropped and every queue is
    /// empty — i.e. the channel is closed and fully drained.
    pub async fn recv(&mut self) -> Option<PrioritizedItem<T>> {
        loop {
            // Try to dequeue in priority order (index 0 = highest).
            {
                let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
                for q in queues.iter_mut() {
                    if let Some(item) = q.pop_front() {
                        return Some(item);
                    }
                }
            }

            // All queues empty — check if senders are gone.
            if self.sender_token.strong_count() == 0 {
                let queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
                if queues.iter().all(|q| q.is_empty()) {
                    return None;
                }
            }

            // Park until a sender enqueues or drops.
            self.notify.notified().await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn higher_priority_dequeued_first() {
        let (tx, mut rx) = priority_channel::<Vec<u8>>(16);

        tx.send(PrioritizedItem {
            priority: NetworkPriority::NORMAL,
            data: vec![1],
        })
        .await
        .unwrap();
        tx.send(PrioritizedItem {
            priority: NetworkPriority::LIFE_SAFETY,
            data: vec![2],
        })
        .await
        .unwrap();
        tx.send(PrioritizedItem {
            priority: NetworkPriority::URGENT,
            data: vec![3],
        })
        .await
        .unwrap();

        let first = rx.recv().await.unwrap();
        assert_eq!(first.priority, NetworkPriority::LIFE_SAFETY);
        assert_eq!(first.data, vec![2]);

        let second = rx.recv().await.unwrap();
        assert_eq!(second.priority, NetworkPriority::URGENT);
        assert_eq!(second.data, vec![3]);

        let third = rx.recv().await.unwrap();
        assert_eq!(third.priority, NetworkPriority::NORMAL);
        assert_eq!(third.data, vec![1]);
    }

    #[tokio::test]
    async fn same_priority_fifo() {
        let (tx, mut rx) = priority_channel::<Vec<u8>>(16);

        tx.send(PrioritizedItem {
            priority: NetworkPriority::NORMAL,
            data: vec![1],
        })
        .await
        .unwrap();
        tx.send(PrioritizedItem {
            priority: NetworkPriority::NORMAL,
            data: vec![2],
        })
        .await
        .unwrap();

        assert_eq!(rx.recv().await.unwrap().data, vec![1]);
        assert_eq!(rx.recv().await.unwrap().data, vec![2]);
    }

    #[tokio::test]
    async fn sender_drop_closes_channel() {
        let (tx, mut rx) = priority_channel::<Vec<u8>>(16);
        tx.send(PrioritizedItem {
            priority: NetworkPriority::NORMAL,
            data: vec![1],
        })
        .await
        .unwrap();
        drop(tx);

        // Should get the queued item.
        assert_eq!(rx.recv().await.unwrap().data, vec![1]);
        // Then None (closed).
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn capacity_limit() {
        let (tx, mut _rx) = priority_channel::<u8>(2);
        tx.send(PrioritizedItem {
            priority: NetworkPriority::NORMAL,
            data: 1,
        })
        .await
        .unwrap();
        tx.send(PrioritizedItem {
            priority: NetworkPriority::NORMAL,
            data: 2,
        })
        .await
        .unwrap();
        // Third should fail (at capacity for NORMAL queue).
        let result = tx
            .send(PrioritizedItem {
                priority: NetworkPriority::NORMAL,
                data: 3,
            })
            .await;
        assert!(result.is_err());

        // But a different priority queue should still accept.
        tx.send(PrioritizedItem {
            priority: NetworkPriority::URGENT,
            data: 4,
        })
        .await
        .unwrap();
    }

    #[test]
    fn priority_index_ordering() {
        assert_eq!(priority_index(NetworkPriority::LIFE_SAFETY), 0);
        assert_eq!(priority_index(NetworkPriority::CRITICAL_EQUIPMENT), 1);
        assert_eq!(priority_index(NetworkPriority::URGENT), 2);
        assert_eq!(priority_index(NetworkPriority::NORMAL), 3);
    }
}
