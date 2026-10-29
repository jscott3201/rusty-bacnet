//! A log page's requests, each charged to the limits.
//!
//! One page of a Trend Log, Trend Log Multiple, Event Log or Audit Log can
//! take several requests: the log's record counts, then the ReadRange, and
//! the counts again when the log moved meanwhile. The page holds its
//! device's turn throughout, so they still go one at a time; the turn pays
//! for the first request and each later one takes a slot of its own, so a
//! page costs what it sends (#2066).
//!
//! Once a Trend Log's first page is answered, [`Engine::log_columns`]
//! reads what it samples, so its values can be named (#2091): one more
//! request, charged the same way, once per read rather than once per page.
//! Asking only after an answer keeps a silent device to one timeout.
//! [`Engine::log_state_texts`] then reads the state texts of the binary
//! and multi-state objects whose states the columns sample (#2122): one
//! request an object, at most
//! [`MAX_STATE_TEXT_OBJECTS`](crate::format::MAX_STATE_TEXT_OBJECTS).
//! Naming never fails a read, but each says, as a [`Naming`], whether a
//! request of its timed out, so a caller that counts timeouts counts it
//! (#2131).

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogRequester, unsigned_value};
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::{Engine, Wanted, timed_out, unreachable};
use crate::format::{LogColumns, StateTexts};
use crate::limits::RequestLimits;

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

    fn of(error: &Error) -> Self {
        if timed_out(error) {
            Self::TimedOut
        } else {
            Self::Answered
        }
    }
}

/// The requester a page reads through: the device's routed read calls,
/// each after a slot (the first under the page's turn).
pub(super) struct PagedLog<'a, T: TransportPort + 'static> {
    pub(super) client: &'a BACnetClient<T>,
    pub(super) limits: &'a RequestLimits,
    pub(super) device: u32,
    pub(super) network: Option<u16>,
    /// Set until the first request, which the page's turn paid for.
    pub(super) first: AtomicBool,
}

impl<T: TransportPort + 'static> PagedLog<'_, T> {
    async fn charge(&self) {
        if !self.first.swap(false, Ordering::Relaxed) {
            self.limits.slot(self.network).await;
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
            .client
            .read_property_from_device(self.device, log, property, None)
            .await?;
        unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        _mac: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        self.charge().await;
        self.client
            .read_range_from_device_with(self.device, request, ReadRangeValidation::Lenient)
            .await
    }
}

impl<T: TransportPort + 'static> Engine<T> {
    /// What each column of `log` on `device` samples, from its
    /// Log_DeviceObjectProperty: one ReadProperty, charged like any other
    /// read, for a Trend Log or Trend Log Multiple, and none for another
    /// log. Naming is best effort: a refused, failed or empty read names no
    /// column, and the values print as the log gives them. The [`Naming`]
    /// says whether the read timed out.
    pub async fn log_columns(&self, device: u32, log: ObjectIdentifier) -> (LogColumns, Naming) {
        if !LogColumns::applies(log.object_type()) {
            return (LogColumns::default(), Naming::Answered);
        }
        match self
            .read_property(
                device,
                log,
                PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
                None,
            )
            .await
        {
            Ok(data) => (LogColumns::decode(&data), Naming::Answered),
            Err(error) => (LogColumns::default(), Naming::of(&error)),
        }
    }

    /// Name the states `columns` sample with their objects' state texts,
    /// read from `device`: one charged request an object (ReadPropertyMultiple,
    /// or ReadProperty each once the device has refused it), each object
    /// once, only objects on `device`, at most
    /// [`MAX_STATE_TEXT_OBJECTS`](crate::format::MAX_STATE_TEXT_OBJECTS).
    /// Best effort: a refused read leaves its columns named as before, and
    /// a device that stops answering, or `out_of_time` turning true, ends
    /// the reading before the next object. The [`Naming`] says whether the
    /// read that ended it timed out.
    pub async fn log_state_texts(
        &self,
        device: u32,
        columns: &mut LogColumns,
        out_of_time: impl Fn() -> bool,
    ) -> Naming {
        let out_of_time = &out_of_time;
        // Atomic rather than a Cell: the MCP server's futures are Send.
        let lost = AtomicBool::new(false);
        let lost = &lost;
        columns
            .read_state_texts(Some(device), |object| async move {
                if out_of_time() {
                    return ControlFlow::Break(());
                }
                let wanted = Wanted::List(StateTexts::properties(object.object_type()));
                match self.read(device, object, wanted).await {
                    Ok(read) => ControlFlow::Continue(StateTexts::from_rows(&read.rows)),
                    Err(error) if unreachable(&error) => {
                        lost.store(timed_out(&error), Ordering::Relaxed);
                        ControlFlow::Break(())
                    }
                    Err(_) => ControlFlow::Continue(StateTexts::default()),
                }
            })
            .await;
        if lost.load(Ordering::Relaxed) {
            Naming::TimedOut
        } else {
            Naming::Answered
        }
    }
}
