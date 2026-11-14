//! A [`Device`]'s read operations: the same requests the client's
//! [`Reader`](crate::client::Reader) sends, one at a time at the device's
//! gate, with the client's memory of an RPM refusal. A following handle
//! checks where its device is before each request ([`Device::target`]),
//! and looks for it again after one times out or finds no route
//! ([`Device::retry`]).

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::log_reader::{LogCursor, LogPage, read_log_page};
use bacnet_services::alarm_event::GetEventInformationAck;
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::{Device, DeviceAddress, Path};
use crate::client::{
    DEVICE_PROPERTIES, DeviceSummary, ListRead, Naming, PagedLog, PropertyRead, Wanted,
    device_error, timed_out, unreachable,
};
use crate::format::{LogColumns, StateTexts};
use crate::limits::Turn;

impl<T: TransportPort + 'static> Device<T> {
    /// The properties `wanted` of `object`: one ReadPropertyMultiple, or
    /// ReadProperty each once the device has refused RPM. A property the
    /// device can't give is an error row, not a failed read.
    pub async fn read(
        &self,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<PropertyRead, Error> {
        sent!(self, |at| self.read_at(at, object, wanted).await)
    }

    async fn read_at(
        &self,
        at: &DeviceAddress,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<PropertyRead, Error> {
        let mut reader = self.client.reader(at, self.instance());
        let read = reader.properties(object, wanted).await;
        self.client.settle(&reader);
        let (rows, via) = read.map_err(device_error)?;
        Ok(PropertyRead { object, rows, via })
    }

    /// One property of `object`, or one element of it (`index` 0 is an
    /// array's length), as its raw application-tagged value.
    pub async fn read_property(
        &self,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<Vec<u8>, Error> {
        sent!(self, |at| {
            let reader = self.client.reader(at, self.instance());
            reader
                .rp(object, property, index)
                .await
                .map_err(device_error)
        })
    }

    /// The device's Object_List: whole, or element by element when the
    /// device can't send it in one answer (at most
    /// [`MAX_OBJECTS`](crate::client::MAX_OBJECTS)). Read by the wildcard
    /// instance while the device's own isn't known.
    pub async fn list_objects(&self) -> Result<ListRead, Error> {
        sent!(self, |at| {
            let reader = self.client.reader(at, self.instance());
            reader.object_list(|_, _| {}).await.map_err(device_error)
        })
    }

    /// The device's [`DEVICE_PROPERTIES`]. A handle that doesn't know the
    /// instance reads it first ([`resolve_instance`](Self::resolve_instance)).
    pub async fn describe_device(&self) -> Result<DeviceSummary, Error> {
        let device = self.resolve_instance().await?;
        let object = ObjectIdentifier::new(ObjectType::DEVICE, device)?;
        let PropertyRead { rows, via, .. } =
            self.read(object, Wanted::List(&DEVICE_PROPERTIES)).await?;
        Ok(DeviceSummary { device, rows, via })
    }

    /// One page of at most `page_size` records of `log` (a Trend Log,
    /// Trend Log Multiple, Event Log or Audit Log), from `cursor`; the page
    /// says where to read next.
    ///
    /// A page can take more than one request (the log's record counts,
    /// then the ReadRange). It holds the device's gate throughout, so they
    /// still go one at a time, and each takes a rate slot of its own.
    pub async fn read_trend(
        &self,
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        sent!(self, |at| {
            let limits = self.client.limits();
            let path = Path::to(self.client.low_level(), at).await?;
            let _turn = limits.turn(at).await;
            let requester = PagedLog {
                client: self.client.low_level(),
                limits,
                path,
                network: at.network(),
                first: AtomicBool::new(true),
            };
            // The requests go along the path, so the link MAC is unused.
            read_log_page(&requester, &[], log, cursor, page_size).await
        })
    }

    /// The path to `at`, and the device's turn there.
    async fn turn_at(&self, at: &DeviceAddress) -> Result<(Path, Turn), Error> {
        let path = Path::to(self.client.low_level(), at).await?;
        Ok((path, self.client.limits().turn(at).await))
    }

    /// One GetEventInformation answer: the objects with an event state
    /// other than normal or with transitions not yet acknowledged. `after`
    /// continues from the last object of the previous answer while that
    /// answer said more events remain.
    pub async fn alarms(
        &self,
        after: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        let client = self.client.low_level();
        sent!(self, |at| {
            let (path, _turn) = self.turn_at(at).await?;
            path.event_information(client, after).await
        })
    }

    /// The device's GetAlarmSummary answer: the objects whose alarms it
    /// reports this way.
    pub async fn alarm_summary(&self) -> Result<GetAlarmSummaryAck, Error> {
        let client = self.client.low_level();
        sent!(self, |at| {
            let (path, _turn) = self.turn_at(at).await?;
            path.alarm_summary(client).await
        })
    }

    /// The device's GetEnrollmentSummary answer for `request`'s filters.
    pub async fn enrollment_summary(
        &self,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        let client = self.client.low_level();
        sent!(self, |at| {
            let (path, _turn) = self.turn_at(at).await?;
            path.enrollment_summary(client, request).await
        })
    }

    /// What each column of `log` samples, from its
    /// Log_DeviceObjectProperty: one ReadProperty, charged like any other
    /// read, for a Trend Log or Trend Log Multiple, and none for another
    /// log. Naming is best effort: a refused, failed or empty read names no
    /// column, and the values print as the log gives them. The [`Naming`]
    /// says whether the read timed out.
    pub async fn log_columns(&self, log: ObjectIdentifier) -> (LogColumns, Naming) {
        if !LogColumns::applies(log.object_type()) {
            return (LogColumns::default(), Naming::Answered);
        }
        match self
            .read_property(log, PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
            .await
        {
            Ok(data) => (LogColumns::decode(&data), Naming::Answered),
            Err(error) => (LogColumns::default(), Naming::of(&error)),
        }
    }

    /// Name the states `columns` sample with their objects' state texts:
    /// one charged request an object (ReadPropertyMultiple, or ReadProperty
    /// each once the device has refused it), each object once, only objects
    /// on this device (all of them a reference names no device for, and
    /// none naming one while the instance isn't known), at most
    /// [`MAX_STATE_TEXT_OBJECTS`](crate::format::MAX_STATE_TEXT_OBJECTS).
    /// Best effort: a refused read leaves its columns named as before, and
    /// a device that stops answering, or `out_of_time` turning true, ends
    /// the reading before the next object. The [`Naming`] says whether the
    /// read that ended it timed out.
    pub async fn log_state_texts(
        &self,
        columns: &mut LogColumns,
        out_of_time: impl Fn() -> bool,
    ) -> Naming {
        let out_of_time = &out_of_time;
        // Atomic rather than a Cell: the MCP server's futures are Send.
        let lost = AtomicBool::new(false);
        let lost = &lost;
        columns
            .read_state_texts(self.instance(), |object| async move {
                if out_of_time() {
                    return ControlFlow::Break(());
                }
                let wanted = Wanted::List(StateTexts::properties(object.object_type()));
                match self.read(object, wanted).await {
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
