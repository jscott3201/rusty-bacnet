use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

use super::TransactionOwner;

const NOT_STARTED: u8 = 0;
const POLLING: u8 = 1;
const ISSUED: u8 = 2;
const FAILED: u8 = 3;

#[derive(Debug)]
pub(crate) enum TerminalResponseAdmission {
    Active(TransactionOwner),
    FinalSegmentSendPolling {
        owner: TransactionOwner,
        issue: FinalSegmentIssue,
    },
    PrematureSegmentedRequestAborted,
    NoTransaction,
}

#[derive(Debug)]
struct FinalSegmentIssueInner {
    state: AtomicU8,
    resolved: Notify,
}

#[derive(Clone, Debug)]
pub(crate) struct FinalSegmentIssue(Arc<FinalSegmentIssueInner>);

impl FinalSegmentIssue {
    pub(super) fn new() -> Self {
        Self(Arc::new(FinalSegmentIssueInner {
            state: AtomicU8::new(NOT_STARTED),
            resolved: Notify::new(),
        }))
    }

    pub(super) fn begin(&self) -> bool {
        self.0
            .state
            .compare_exchange(NOT_STARTED, POLLING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn is_polling(&self) -> bool {
        self.0.state.load(Ordering::Acquire) == POLLING
    }

    pub(super) fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn resolve(&self, state: u8) {
        self.0.state.store(state, Ordering::Release);
        self.0.resolved.notify_waiters();
    }

    pub(crate) async fn wait_until_polled(&self) {
        loop {
            let resolved = self.0.resolved.notified();
            if !self.is_polling() {
                return;
            }
            resolved.await;
        }
    }
}

#[derive(Debug)]
pub(crate) struct FinalSegmentSendToken {
    pub(super) issue: FinalSegmentIssue,
    pub(super) resolved: bool,
}

impl FinalSegmentSendToken {
    pub(super) fn issued(&mut self) {
        self.issue.resolve(ISSUED);
        self.resolved = true;
    }
}

impl Drop for FinalSegmentSendToken {
    fn drop(&mut self) {
        if !self.resolved {
            self.issue.resolve(FAILED);
        }
    }
}
