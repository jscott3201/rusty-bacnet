//! A log page's requests, each charged to the limits.
//!
//! One page of a Trend Log, Trend Log Multiple, Event Log or Audit Log can
//! take several requests: the log's record counts, then the ReadRange, and
//! the counts again when the log moved meanwhile. The page holds its
//! device's turn throughout, so they still go one at a time; the turn pays
//! for the first request and each later one takes a slot of its own, so a
//! page costs what it sends (#2066).
//!
//! Once a Trend Log's first page is answered, [`Device::log_columns`]
//! reads what it samples, so its values can be named (#2091): one more
//! request, charged the same way, once per read rather than once per page.
//! Asking only after an answer keeps a silent device to one timeout.
//! [`Device::log_state_texts`] then reads the state texts of the binary
//! and multi-state objects whose states the columns sample (#2122): one
//! request an object, at most
//! [`MAX_STATE_TEXT_OBJECTS`](crate::format::MAX_STATE_TEXT_OBJECTS).
//! Naming never fails a read, but each says, as a [`Naming`], whether a
//! request of its timed out, so a caller that counts timeouts counts it
//! (#2131).

use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogRequester, unsigned_value};
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::timed_out;
use crate::device::Path;
use crate::limits::RequestLimits;

#[cfg(doc)]
use crate::device::Device;

/// How a best-effort naming read ended: it never fails the read it names,
/// but a request it lost to a timeout is still a timeout (#2131).
#[must_use = "a timed-out naming request is a timeout the caller may count"]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Naming {
    /// Every request sent was answered, or refused, or none was sent.
    #[default]
    Answered,
    /// A request timed out ([`timed_out`]), and the
    /// naming stopped there.
    TimedOut,
}

impl Naming {
    /// Whether a request timed out.
    pub fn timed_out(self) -> bool {
        self == Self::TimedOut
    }

    pub(crate) fn of(error: &Error) -> Self {
        if timed_out(error) {
            Self::TimedOut
        } else {
            Self::Answered
        }
    }
}

/// The requester a page reads through: requests along the device's path,
/// each after a slot (the first under the page's turn).
pub(crate) struct PagedLog<'a, T: TransportPort + 'static> {
    pub(crate) client: &'a BACnetClient<T>,
    pub(crate) limits: &'a RequestLimits,
    pub(crate) path: Path,
    pub(crate) network: u16,
    /// Set until the first request, which the page's turn paid for.
    pub(crate) first: AtomicBool,
}

impl<T: TransportPort + 'static> PagedLog<'_, T> {
    async fn charge(&self) {
        if !self.first.swap(false, Ordering::Relaxed) {
            self.limits.slot(Some(self.network)).await;
        }
    }
}

impl<T: TransportPort + 'static> LogRequester for PagedLog<'_, T> {
    async fn read_unsigned(
        &self,
        _mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        self.charge().await;
        let ack = self
            .path
            .read_property(self.client, log, property, None)
            .await?;
        unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        _mac: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        self.charge().await;
        self.path.read_range_lenient(self.client, request).await
    }
}
