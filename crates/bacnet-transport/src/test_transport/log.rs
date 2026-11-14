//! The frames a test transport was asked to send, and the log that holds them.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu};
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::Notify;

use crate::port::DataAttribute;

/// One NPDU handed to the transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentFrame {
    /// The NPDU bytes exactly as sent.
    pub npdu: Bytes,
    /// Destination MAC. Empty for a broadcast.
    pub mac: MacAddr,
    /// Whether this was `send_broadcast`.
    pub broadcast: bool,
    /// The data attributes the send carried: empty for a send that named none.
    pub attributes: Vec<DataAttribute>,
}

impl SentFrame {
    /// Decode the NPDU, panicking on malformed bytes.
    pub fn decode_npdu(&self) -> Npdu {
        decode_npdu(self.npdu.clone()).expect("sent frame should decode as NPDU")
    }

    /// Decode the APDU the NPDU carries, panicking on malformed bytes.
    pub fn apdu(&self) -> Apdu {
        decode_apdu(self.decode_npdu().payload).expect("sent NPDU should carry an APDU")
    }
}

/// Shared, cloneable log of recorded sends.
#[derive(Clone, Default)]
pub struct SendLog {
    inner: Arc<LogInner>,
}

#[derive(Default)]
struct LogInner {
    frames: Mutex<Vec<SentFrame>>,
    pushed: Notify,
}

impl SendLog {
    /// Lock the underlying frames for in-place inspection or mutation. A
    /// poisoned lock is recovered so a panicking assertion in one test thread
    /// does not bury its failure under later pushes from the stack.
    pub fn lock(&self) -> MutexGuard<'_, Vec<SentFrame>> {
        self.inner
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Append a frame. The transport does this itself; hooks may record extra.
    pub fn push(&self, frame: SentFrame) {
        self.lock().push(frame);
        self.inner.pushed.notify_waiters();
    }

    /// How many frames are recorded.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no frame is recorded.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Snapshot of every recorded frame.
    pub fn frames(&self) -> Vec<SentFrame> {
        self.lock().clone()
    }

    /// The frame at `index`, panicking when absent.
    pub fn frame(&self, index: usize) -> SentFrame {
        let frame = self.lock().get(index).cloned();
        frame.unwrap_or_else(|| {
            panic!(
                "no sent frame at index {index}; the log holds {}",
                self.len()
            )
        })
    }

    /// Remove and return every recorded frame.
    pub fn take(&self) -> Vec<SentFrame> {
        std::mem::take(&mut *self.lock())
    }

    /// Forget every recorded frame.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Recorded unicast frames, in order.
    pub fn unicasts(&self) -> Vec<SentFrame> {
        self.lock()
            .iter()
            .filter(|f| !f.broadcast)
            .cloned()
            .collect()
    }

    /// Recorded broadcast frames, in order.
    pub fn broadcasts(&self) -> Vec<SentFrame> {
        self.lock()
            .iter()
            .filter(|f| f.broadcast)
            .cloned()
            .collect()
    }

    /// NPDU bytes of every recorded frame.
    pub fn npdus(&self) -> Vec<Bytes> {
        self.lock().iter().map(|f| f.npdu.clone()).collect()
    }

    /// Wait until at least `len` frames are recorded. Callers add a timeout.
    pub async fn wait_for_len(&self, len: usize) {
        loop {
            let pushed = self.inner.pushed.notified();
            if self.len() >= len {
                return;
            }
            pushed.await;
        }
    }
}
