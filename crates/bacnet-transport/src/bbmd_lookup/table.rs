//! A table of host names for tests (#1852, #2308): a [`BbmdResolver`]
//! that answers each name as set, and counts its calls and the most that
//! ran at once. B/IP's and B/IPv6's name tests share it.

use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::{BbmdLookup, BbmdResolver};

/// What the table answers for a name.
#[derive(Clone)]
pub(crate) enum Answer {
    Addresses(Vec<IpAddr>),
    Fails(io::ErrorKind),
    Hangs,
    /// These addresses, once the gate opens.
    Gated(Arc<Notify>, Vec<IpAddr>),
    Panics,
}

impl Answer {
    /// These addresses, of either family.
    pub(crate) fn addresses<A: Into<IpAddr>>(addresses: impl IntoIterator<Item = A>) -> Self {
        Self::Addresses(addresses.into_iter().map(Into::into).collect())
    }

    /// These addresses, once `gate` opens.
    pub(crate) fn gated<A: Into<IpAddr>>(
        gate: &Arc<Notify>,
        addresses: impl IntoIterator<Item = A>,
    ) -> Self {
        let addresses = addresses.into_iter().map(Into::into).collect();
        Self::Gated(Arc::clone(gate), addresses)
    }
}

/// A resolver that answers from a table, and counts its calls and the most
/// that ran at once. A name not in the table fails as not found.
#[derive(Default)]
pub(crate) struct Names {
    answers: Mutex<HashMap<String, Answer>>,
    calls: AtomicUsize,
    running: Arc<AtomicUsize>,
    most_running: Arc<AtomicUsize>,
}

/// One call running, until dropped.
struct Running(Arc<AtomicUsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Names {
    pub(crate) fn answer(&self, host: &str, answer: Answer) {
        self.answers.lock().unwrap().insert(host.into(), answer);
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn most_running(&self) -> usize {
        self.most_running.load(Ordering::SeqCst)
    }
}

impl BbmdResolver for Names {
    fn resolve(&self, host: &str) -> BbmdLookup {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answers.lock().unwrap().get(host).cloned();
        let (running, most) = (Arc::clone(&self.running), Arc::clone(&self.most_running));
        Box::pin(async move {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            let _running = Running(running);
            match answer.unwrap_or(Answer::Fails(io::ErrorKind::NotFound)) {
                Answer::Addresses(addresses) => Ok(addresses),
                Answer::Fails(kind) => Err(io::Error::from(kind)),
                Answer::Hangs => std::future::pending().await,
                Answer::Gated(gate, addresses) => {
                    gate.notified().await;
                    Ok(addresses)
                }
                Answer::Panics => panic!("the test resolver panics"),
            }
        })
    }
}
