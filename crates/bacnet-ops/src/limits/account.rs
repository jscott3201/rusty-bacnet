//! What one caller's requests cost, and the caller's own rate.
//!
//! A front end serving several callers (the MCP server's principals, #2066)
//! runs each call under an [`Account`] with [`charged`]. Every request the
//! call sends through [`RequestLimits`](super::RequestLimits) then counts
//! against it, with the time it waited for the limits, and first takes a
//! token from the caller's [`TokenBucket`], when the account has one. The
//! account rides in a task-local, so the engine's read paths charge it
//! without passing it along; a request made outside [`charged`] (the TUI's)
//! charges nothing.

use std::future::Future;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

tokio::task_local! {
    static ACCOUNT: Arc<Account>;
}

/// One caller's tally: the requests it sent and how long they waited for
/// the limits, and the caller's token bucket, if it has one.
#[derive(Debug, Default)]
pub struct Account {
    requests: AtomicU32,
    throttled_us: AtomicU64,
    bucket: Option<Arc<TokenBucket>>,
}

impl Account {
    /// An empty tally, whose requests also take tokens from `bucket`.
    pub fn new(bucket: Option<Arc<TokenBucket>>) -> Self {
        Self {
            requests: AtomicU32::new(0),
            throttled_us: AtomicU64::new(0),
            bucket,
        }
    }

    /// Requests counted so far.
    pub fn requests(&self) -> u32 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Time the requests spent waiting for a device's gate, the caller's
    /// bucket, a network's rate or the global rate.
    pub fn throttled(&self) -> Duration {
        Duration::from_micros(self.throttled_us.load(Ordering::Relaxed))
    }

    pub(super) fn count_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Take back a request counted for a send that never left.
    fn uncount_request(&self) {
        // The refunded request was counted just before, by this call's
        // own task, so the count is above zero here.
        if self.requests.load(Ordering::Relaxed) > 0 {
            self.requests.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub(super) fn add_wait(&self, waited: Duration) {
        let micros = u64::try_from(waited.as_micros()).unwrap_or(u64::MAX);
        self.throttled_us.fetch_add(micros, Ordering::Relaxed);
    }

    pub(super) fn bucket(&self) -> Option<&TokenBucket> {
        self.bucket.as_deref()
    }
}

/// Run `future` with every request it sends charged to `account`.
pub async fn charged<F: Future>(account: Arc<Account>, future: F) -> F::Output {
    ACCOUNT.scope(account, future).await
}

/// Take back the last request this task's call was charged for, when its
/// send failed before anything left (the transport refused it).
pub fn refund() {
    if let Some(account) = current() {
        account.uncount_request();
    }
}

/// The account of the call running on this task, if any.
pub(super) fn current() -> Option<Arc<Account>> {
    ACCOUNT.try_with(Arc::clone).ok()
}

/// A token bucket: at most `burst` requests at once, refilled at
/// `per_second`. A caller that finds it empty waits for its token, which
/// is booked as it starts waiting, so callers are served in turn and a
/// dropped wait still spends its token.
#[derive(Debug)]
pub struct TokenBucket {
    per_second: f64,
    burst: f64,
    state: Mutex<Tokens>,
}

#[derive(Debug)]
struct Tokens {
    /// Tokens left; below zero, tokens already promised to waiters.
    left: f64,
    at: Instant,
}

impl TokenBucket {
    /// A bucket of `burst` tokens (at least one), refilled at `per_second`;
    /// 0 means no limit.
    pub fn new(per_second: u32, burst: u32) -> Self {
        let burst = f64::from(burst.max(1));
        Self {
            per_second: f64::from(per_second),
            burst,
            state: Mutex::new(Tokens {
                left: burst,
                at: Instant::now(),
            }),
        }
    }

    /// Take a token, waiting for one when the bucket is empty.
    pub async fn take(&self) {
        if self.per_second == 0.0 {
            return;
        }
        let wait = {
            let mut tokens = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let refill = now.saturating_duration_since(tokens.at).as_secs_f64() * self.per_second;
            tokens.left = (tokens.left + refill).min(self.burst) - 1.0;
            tokens.at = now;
            (tokens.left < 0.0).then(|| Duration::from_secs_f64(-tokens.left / self.per_second))
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full bucket gives its burst at once, then one token every
    /// `1 / per_second`; after an idle spell it is full again.
    #[tokio::test(start_paused = true)]
    async fn a_bucket_gives_its_burst_then_its_rate() {
        let bucket = TokenBucket::new(5, 3);
        let start = Instant::now();
        let mut times = Vec::new();
        for _ in 0..5 {
            bucket.take().await;
            times.push((Instant::now() - start).as_millis());
        }
        assert_eq!(times, [0, 0, 0, 200, 400]);
        tokio::time::sleep(Duration::from_secs(10)).await;
        let before = Instant::now();
        for _ in 0..3 {
            bucket.take().await;
        }
        assert_eq!(Instant::now(), before, "refilled to the burst, no more");
        bucket.take().await;
        assert_eq!(Instant::now() - before, Duration::from_millis(200));
        let unlimited = TokenBucket::new(0, 1);
        let before = Instant::now();
        for _ in 0..100 {
            unlimited.take().await;
        }
        assert_eq!(Instant::now(), before);
    }

    /// A charged future sees its account; one outside sees none.
    #[tokio::test]
    async fn the_account_rides_with_the_call() {
        assert!(current().is_none());
        let account = Arc::new(Account::new(None));
        charged(Arc::clone(&account), async {
            current().expect("an account").count_request();
        })
        .await;
        assert_eq!(account.requests(), 1);
    }
}
