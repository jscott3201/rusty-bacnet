//! One bounded FIFO writer queue, with separate ordinary and reply authority.
use super::direct_membership::Membership;
use crate::port::DataAttribute;
use crate::sc_frame::{ScFunction, ScMessage, encode_sc_message};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU16, Ordering},
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub(crate) const WRITE_CAPACITY: usize = 64;
pub(crate) struct DirectWrite {
    pub(crate) bytes: Bytes,
    pub(crate) done: oneshot::Sender<Result<(), Error>>,
    pub(crate) scopes: [Option<crate::port::ResponseScopeGuard>; 2],
    pub(crate) deadline: tokio::time::Instant,
    pub(crate) started: Arc<AtomicBool>,
}
impl DirectWrite {
    pub(crate) fn can_start(&self) -> bool {
        tokio::time::Instant::now() < self.deadline
            && !self.done.is_closed()
            && self
                .scopes
                .iter()
                .flatten()
                .all(crate::port::ResponseScopeGuard::is_open)
    }
    pub(crate) fn mark_started(&self) {
        self.started.store(true, Ordering::Release);
    }
}

/// Only Unavailable permits a fresh route decision; capacity and uncertain
/// writes must not silently retry the same payload through another socket.
#[derive(Debug)]
pub(crate) enum DirectSendError {
    Unavailable,
    Capacity,
    Failed(Error),
}
impl DirectSendError {
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Unavailable => crate::direct_response::unavailable(),
            Self::Capacity => Error::Transport(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "direct socket write queue full",
            )),
            Self::Failed(error) => error,
        }
    }
}

#[derive(Clone)]
pub(crate) struct DirectEgress {
    member: Weak<Membership>,
    pub(crate) send: mpsc::Sender<DirectWrite>,
    pub(crate) next_message: Arc<AtomicU16>,
    pub(crate) limits: (u16, u16),
    pub(crate) wait: Duration,
}
impl DirectEgress {
    pub(crate) fn new(
        member: Weak<Membership>,
        limits: (u16, u16),
        wait: Duration,
    ) -> (Self, mpsc::Receiver<DirectWrite>) {
        let (send, recv) = mpsc::channel(WRITE_CAPACITY);
        (
            Self {
                member,
                send,
                next_message: Arc::new(AtomicU16::new(1)),
                limits,
                wait,
            },
            recv,
        )
    }
    pub(crate) async fn send_npdu(
        &self,
        npdu: &[u8],
        attributes: &[DataAttribute],
    ) -> Result<(), DirectSendError> {
        if npdu.len() > usize::from(self.limits.1) {
            return Err(DirectSendError::Failed(Error::SendTooLong {
                length: npdu.len(),
                limit: usize::from(self.limits.1),
            }));
        }
        let frame = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_message.fetch_add(1, Ordering::Relaxed),
            originating_vmac: None,
            destination_vmac: None,
            dest_options: vec![],
            data_options: super::data_attributes::to_data_options(attributes)
                .map_err(DirectSendError::Failed)?,
            payload: Bytes::copy_from_slice(npdu),
        };
        let mut bytes = BytesMut::new();
        encode_sc_message(&mut bytes, &frame);
        self.send_frame(bytes.freeze()).await
    }
    pub(crate) async fn send_frame(&self, bytes: Bytes) -> Result<(), DirectSendError> {
        if bytes.len() > usize::from(self.limits.0) {
            return Err(DirectSendError::Failed(Error::SendTooLong {
                length: bytes.len(),
                limit: usize::from(self.limits.0),
            }));
        }
        let started = Arc::new(AtomicBool::new(false));
        let (done, receive) = oneshot::channel();
        let deadline = tokio::time::Instant::now()
            .checked_add(self.wait)
            .ok_or_else(|| DirectSendError::Failed(crate::direct_response::unavailable()))?;
        self.member
            .upgrade()
            .and_then(|member| {
                member.with_current(|| {
                    self.send.try_send(DirectWrite {
                        bytes,
                        done,
                        scopes: [None, None],
                        deadline,
                        started: started.clone(),
                    })
                })
            })
            .ok_or(DirectSendError::Unavailable)?
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DirectSendError::Capacity,
                mpsc::error::TrySendError::Closed(_) => {
                    DirectSendError::Failed(crate::direct_response::unavailable())
                }
            })?;
        match tokio::time::timeout_at(deadline, receive).await {
            Ok(Ok(Ok(()))) => Ok(()),
            result => {
                // A queued operation cancelled before start cannot run later:
                // its receiver closes and the writer checks both it and deadline.
                let retired = self.member.upgrade().is_none_or(|m| !m.is_current());
                if !started.load(Ordering::Acquire) && retired {
                    Err(DirectSendError::Unavailable)
                } else {
                    let error = match result {
                        Ok(Ok(Err(error))) => error,
                        _ => crate::direct_response::unavailable(),
                    };
                    Err(DirectSendError::Failed(error))
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "direct_egress_tests.rs"]
mod tests;
