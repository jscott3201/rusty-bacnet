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
use crate::client::batch::{self, BatchRow, Batched, Missing};
use crate::client::{
    DEVICE_PROPERTIES, DeviceSummary, ListRead, Naming, PagedLog, ReadVia, Wanted, device_error,
};
use crate::error::OpsError;
use crate::format::{Floats, LogColumns, PropertyError, RawRow, StateTexts, Value, ValueFormat};
use crate::limits::Turn;
use crate::reading::{IntoProperty, PropertyRef, RawReading, Readings, readings};

impl<T: TransportPort + 'static> Device<T> {
    /// One property of one object, or one element of an array property
    /// (index 0 is an array's size): one ReadProperty. `property` is a
    /// [`PropertyRef`], text such as `"ai:1 pv"` or a tuple
    /// ([`IntoProperty`]). The value names no units or state texts, which
    /// the one property doesn't hold; [`read_many`](Self::read_many) with
    /// the object's Units or state texts too names them.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for text that doesn't name a property (nothing
    /// is sent), and the device's refusal of the property as
    /// [`OpsError::Refused`].
    pub async fn read(&self, property: impl IntoProperty) -> Result<Value, OpsError> {
        let wanted = property.into_property()?;
        let PropertyRef {
            object,
            property,
            index,
        } = wanted;
        let raw = sent!(self, |at| {
            let reader = self.client.reader(at, self.instance());
            reader
                .rp(object, property, index)
                .await
                .map_err(device_error)
        })?;
        let texts = StateTexts::default();
        let format = ValueFormat::new(object.object_type(), &texts, Floats::Exact).whole_lists();
        Ok(Value::new(&format, property, index, raw, None))
    }

    /// Several properties, of any of the device's objects, in batches:
    /// ReadPropertyMultiple of up to 16 objects and 32 properties a request,
    /// properties of one object named one after another going in one
    /// access specification; ReadProperty each once the device has refused
    /// RPM. A batch whose answer would be too large is asked again an
    /// object at a time, and any other batch the device fails as a whole is
    /// read by ReadProperty, so one bad object can't put errors on its
    /// neighbours. The readings come in the order asked for, a value named
    /// with its object's Units and state texts when the same call reads
    /// them.
    ///
    /// It never fails as a whole. A property the device refuses, or one of
    /// an object it doesn't have, is an error in its own row. A device that
    /// stops answering ends the read with what was read before it, and
    /// [`Readings::stopped`] says why; so does a device that can't be
    /// placed, or a property whose text doesn't parse (then nothing is
    /// sent). A following handle whose read stopped before anything was
    /// read looks for its device as any other read does, and reads again
    /// where it moved to. Every property of an object (ALL) is
    /// [`read_object`](Self::read_object)'s.
    pub async fn read_many<P: IntoProperty>(
        &self,
        properties: impl IntoIterator<Item = P>,
    ) -> Readings {
        let mut wanted = Vec::new();
        for property in properties {
            match property.into_property() {
                Ok(property) => wanted.push(property),
                Err(error) => return Readings::nothing_read(error),
            }
        }
        if wanted.is_empty() {
            return Readings::default();
        }
        let rows = batch_rows(&wanted);
        let mut address = match self.target().await {
            Ok(address) => address,
            Err(error) => return Readings::nothing_read(OpsError::from(error)),
        };
        let mut read = self.batch_at(&address, &rows).await;
        if read.rows.is_empty()
            && let Some(error) = read.stopped.take()
        {
            let stopped = Err::<(), _>(error);
            match self.retry(&address, &stopped).await {
                Ok(Some(to)) => {
                    read = self.batch_at(&to, &rows).await;
                    address = to;
                }
                Ok(None) => read.stopped = stopped.err(),
                Err(error) => return Readings::nothing_read(OpsError::from(error)),
            }
        }
        let via = read.via();
        let stopped = read.stopped.map(|error| OpsError::at(&address, error));
        Readings {
            readings: readings(raw_readings(&rows, read.rows)),
            via,
            stopped,
        }
    }

    /// `rows` read by batches at `at`, remembering a refusal of RPM.
    async fn batch_at(&self, at: &DeviceAddress, rows: &[BatchRow<usize>]) -> Batched<usize> {
        let client = &self.client;
        let rpm = !client.refused_rpm(at);
        let read = batch::read(client.low_level(), client.limits(), at, rows, rpm).await;
        if read.refused_rpm {
            client.refuse_rpm(at);
        }
        read
    }

    /// The properties `wanted` of `object`: one ReadPropertyMultiple, or
    /// ReadProperty each once the device has refused RPM. For
    /// [`Wanted::All`], by ReadProperty the object's Property_List names
    /// them. Each value is named with the object's Units and state texts
    /// when the read holds them.
    ///
    /// It never fails as a whole: a property the device can't give is an
    /// error row, and a device that fails the read, or stops answering
    /// part way through it, leaves no readings and says why in
    /// [`Readings::stopped`].
    pub async fn read_object(&self, object: ObjectIdentifier, wanted: Wanted<'_>) -> Readings {
        match self.rows(object, wanted).await {
            Ok((rows, via)) => Readings {
                readings: readings(rows.into_iter().map(|row| (object, None, row)).collect()),
                via: Some(via),
                stopped: None,
            },
            Err(error) => Readings::nothing_read(error),
        }
    }

    /// [`read_object`](Self::read_object)'s rows, undecoded.
    pub(crate) async fn rows(
        &self,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<(Vec<RawRow>, ReadVia), OpsError> {
        sent!(self, |at| self.rows_at(at, object, wanted).await)
    }

    async fn rows_at(
        &self,
        at: &DeviceAddress,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<(Vec<RawRow>, ReadVia), Error> {
        let mut reader = self.client.reader(at, self.instance());
        let read = reader.properties(object, wanted).await;
        self.client.settle(&reader);
        read.map_err(device_error)
    }

    /// The device's Object_List: whole, or element by element when the
    /// device can't send it in one answer (at most
    /// [`MAX_OBJECTS`](crate::client::MAX_OBJECTS)). Read by the wildcard
    /// instance while the device's own isn't known.
    pub async fn list_objects(&self) -> Result<ListRead, OpsError> {
        sent!(self, |at| {
            let reader = self.client.reader(at, self.instance());
            reader.object_list(|_, _| {}).await.map_err(device_error)
        })
    }

    /// The device's [`DEVICE_PROPERTIES`]. A handle that doesn't know the
    /// instance reads it first ([`resolve_instance`](Self::resolve_instance)).
    pub async fn describe_device(&self) -> Result<DeviceSummary, OpsError> {
        let device = self.resolve_instance().await?;
        let object = ObjectIdentifier::new(ObjectType::DEVICE, device)?;
        let (rows, via) = self.rows(object, Wanted::List(&DEVICE_PROPERTIES)).await?;
        let readings = readings(rows.into_iter().map(|row| (object, None, row)).collect());
        Ok(DeviceSummary {
            device,
            readings,
            via,
        })
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
    ) -> Result<LogPage, OpsError> {
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
    ) -> Result<GetEventInformationAck, OpsError> {
        let client = self.client.low_level();
        sent!(self, |at| {
            let (path, _turn) = self.turn_at(at).await?;
            path.event_information(client, after).await
        })
    }

    /// The device's GetAlarmSummary answer: the objects whose alarms it
    /// reports this way.
    pub async fn alarm_summary(&self) -> Result<GetAlarmSummaryAck, OpsError> {
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
    ) -> Result<GetEnrollmentSummaryAck, OpsError> {
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
            .read((log, PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY))
            .await
        {
            Ok(value) => (LogColumns::decode(&value.raw), Naming::Answered),
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
                match self.rows(object, wanted).await {
                    Ok((rows, _)) => ControlFlow::Continue(StateTexts::from_rows(&rows)),
                    Err(error) if error.unreachable() => {
                        lost.store(error.timed_out(), Ordering::Relaxed);
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

/// The batch rows of `wanted`: one an object, each holding the properties
/// named for it one after another, keyed by the position of its first.
fn batch_rows(wanted: &[PropertyRef]) -> Vec<BatchRow<usize>> {
    let mut rows: Vec<BatchRow<usize>> = Vec::new();
    for (at, property) in wanted.iter().enumerate() {
        match rows.last_mut() {
            Some(row) if row.object == property.object => {
                row.wanted.push((property.property, property.index));
                row.one_by_one += 1;
            }
            _ => rows.push(BatchRow {
                key: at,
                object: property.object,
                wanted: vec![(property.property, property.index)],
                one_by_one: 1,
            }),
        }
    }
    rows
}

/// The rows a batch read as raw readings, each property with the element
/// it asked for. An answer's properties are matched to the ones asked for
/// in order, by property; one the answer left out reads as
/// [`PropertyError::NoValue`], and so does each property of a row it left
/// out.
fn raw_readings(
    asked: &[BatchRow<usize>],
    read: Vec<(usize, Result<Vec<RawRow>, Missing>)>,
) -> Vec<RawReading> {
    let mut out = Vec::new();
    for (row, (_, answer)) in asked.iter().zip(read) {
        let mut answer: Vec<Option<RawRow>> = answer
            .map(|rows| rows.into_iter().map(Some).collect())
            .unwrap_or_default();
        for &(property, index) in &row.wanted {
            let found = answer
                .iter_mut()
                .find(|slot| slot.as_ref().is_some_and(|(p, _)| *p == property))
                .and_then(Option::take);
            let value = found.map_or(Err(PropertyError::NoValue), |(_, value)| value);
            out.push((row.object, index, (property, value)));
        }
    }
    out
}
