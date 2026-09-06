use std::sync::Arc;

use bacnet_encoding::apdu::SegmentAck as SegmentAckPdu;
use bacnet_types::MacAddr;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use tokio::sync::Notify;
use tokio::sync::{mpsc, Mutex};

use crate::tsm::{TransactionOwner, Tsm};

pub(super) struct TransactionCleanup {
    pub(super) mac: MacAddr,
    pub(super) invoke_id: u8,
    pub(super) owner: TransactionOwner,
    pub(super) cancel_tsm: bool,
    pub(super) seg_ack_sender: Option<mpsc::Sender<SegmentAckPdu>>,
}

pub(super) struct TransactionGuard {
    tsm: Arc<Mutex<Tsm>>,
    cleanup_tx: mpsc::UnboundedSender<TransactionCleanup>,
    cleanup: Option<TransactionCleanup>,
}

impl TransactionGuard {
    pub(super) fn new(
        tsm: Arc<Mutex<Tsm>>,
        cleanup_tx: mpsc::UnboundedSender<TransactionCleanup>,
        mac: MacAddr,
        invoke_id: u8,
        owner: TransactionOwner,
        seg_ack_sender: Option<mpsc::Sender<SegmentAckPdu>>,
    ) -> Self {
        Self {
            tsm,
            cleanup_tx,
            cleanup: Some(TransactionCleanup {
                mac,
                invoke_id,
                owner,
                cancel_tsm: true,
                seg_ack_sender,
            }),
        }
    }

    pub(super) fn mark_completed(&mut self) {
        self.cleanup = None;
    }
}

impl Drop for TransactionGuard {
    fn drop(&mut self) {
        let Some(cleanup) = self.cleanup.take() else {
            return;
        };
        if let Err(error) = self.cleanup_tx.send(cleanup) {
            let cleanup = error.0;
            if let Ok(mut tsm) = self.tsm.try_lock() {
                tsm.cancel_transaction_for_owner(&cleanup.mac, cleanup.invoke_id, &cleanup.owner);
            }
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct SegmentedPostWaitCleanupHook {
    enabled: AtomicBool,
    reached: Notify,
    release: Notify,
}

#[cfg(test)]
impl SegmentedPostWaitCleanupHook {
    pub(super) fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    pub(super) async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    pub(super) fn release(&self) {
        self.release.notify_one();
    }

    pub(super) async fn pause_if_enabled(&self) {
        if self.enabled.swap(false, Ordering::SeqCst) {
            self.reached.notify_one();
            self.release.notified().await;
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct SegmentedCleanupHook {
    delay_next: AtomicBool,
    reached: Notify,
    release: Notify,
    processed: Notify,
    last_removed: AtomicBool,
}

#[cfg(test)]
impl SegmentedCleanupHook {
    pub(super) fn delay_next(&self) {
        self.delay_next.store(true, Ordering::SeqCst);
    }

    pub(super) async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    pub(super) fn release(&self) {
        self.release.notify_one();
    }

    pub(super) async fn pause_if_enabled(&self) {
        if self.delay_next.swap(false, Ordering::SeqCst) {
            self.reached.notify_one();
            self.release.notified().await;
        }
    }

    pub(super) fn record_processed(&self, removed: bool) {
        self.last_removed.store(removed, Ordering::SeqCst);
        self.processed.notify_one();
    }

    pub(super) async fn wait_processed(&self) -> bool {
        self.processed.notified().await;
        self.last_removed.load(Ordering::SeqCst)
    }
}
