//! A [`Device`]'s read operations: the same requests the client's
//! [`Reader`](crate::raw::Reader) sends, one at a time at the device's
//! gate, with the client's memory of an RPM refusal. A following handle
//! checks where its device is before each request ([`Device::target`]),
//! and looks for it again after one times out or finds no route
//! ([`Device::retry`]).

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::Station;
use bacnet_client::log_reader::{LogCursor, LogPage, read_log_page};
use bacnet_services::alarm_event::GetEventInformationAck;
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::{Device, DeviceAddress, path_to};
use crate::client::batch::{self, BatchRow, Batched, Missing};
use crate::client::{
    DEVICE_PROPERTIES, DeviceSummary, Halt, ListRead, Naming, PagedLog, ReadVia, RpmFailure,
    Wanted, device_error, failed_locally, rpm_failure, unreachable,
};
use crate::error::OpsError;
use crate::format::{Floats, LogColumns, PropertyError, RawRow, StateTexts, Value, ValueFormat};
use crate::limits::Turn;
use crate::reading::{
    IntoProperty, ObjectRead, PropertyRef, RawReading, Readings, one_property, readings,
};

impl<T: TransportPort + 'static> Device<T> {
    /// One property of one object, or one element of an array property
    /// (index 0 is an array's size): one ReadProperty. `property` is a
    /// [`PropertyRef`], text such as `"ai:1 pv"` or a tuple
    /// ([`IntoProperty`]). Its text names no units and no state, since the
    /// one property holds neither: [`read_many`](Self::read_many) with the
    /// object's Units, Active_Text and Inactive_Text, or State_Text, in the
    /// same call names them.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for text that doesn't name one property, ALL,
    /// REQUIRED and OPTIONAL included ([`read_object`](Self::read_object)
    /// reads those), with nothing sent; and the device's refusal of the
    /// property as [`OpsError::Refused`].
    pub async fn read(&self, property: impl IntoProperty) -> Result<Value, OpsError> {
        let PropertyRef {
            object,
            property,
            index,
        } = one_property(property.into_property()?)?;
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
    /// neighbours. The readings come in the order asked for. A value's text
    /// names its object's Units (after a whole property's number) and its
    /// state by the object's Active_Text and Inactive_Text, or State_Text,
    /// when the same call reads them.
    ///
    /// A property the device refuses, or one of an object it doesn't have,
    /// is an error in its own row, and so is one its answer leaves out
    /// ([`PropertyError::NoValue`]). A device that stops answering ends the
    /// read with what was read before it, an object's properties read
    /// before it included, and [`Readings::stopped`] says so. A following
    /// handle whose read stopped before anything was read looks for its
    /// device as any other read does, and reads again where it moved to.
    ///
    /// # Errors
    ///
    /// Whatever is decided before a request is sent, with nothing sent:
    /// text that doesn't name one property, ALL, REQUIRED and OPTIONAL
    /// included ([`read_object`](Self::read_object) reads those)
    /// ([`OpsError::Parse`]); and a device that can't be placed, an
    /// instance two addresses claim, or a routed address with no route
    /// ([`OpsError::UnknownDevice`], [`OpsError::Ambiguous`]). And a
    /// request the client or the link failed itself (a link that refuses
    /// the send, say; [`OpsError::Client`], #2610) when nothing had been
    /// read yet: a read fails only when nothing could be read. After the
    /// first rows it ends with them, the error in [`Readings::stopped`].
    pub async fn read_many<P: IntoProperty>(
        &self,
        properties: impl IntoIterator<Item = P>,
    ) -> Result<Readings, OpsError> {
        let wanted = properties
            .into_iter()
            .map(|property| one_property(property.into_property()?))
            .collect::<Result<Vec<_>, _>>()?;
        if wanted.is_empty() {
            return Ok(Readings::default());
        }
        let rows = batch_rows(&wanted);
        let mut address = self.target().await?;
        let mut read = self.batch_at(&address, &rows).await?;
        if read.rows.is_empty()
            && let Some(error) = read.stopped.take()
        {
            let stopped = Err::<(), _>(error);
            match self.retry(&address, &stopped).await? {
                Some(to) => {
                    read = self.batch_at(&to, &rows).await?;
                    address = to;
                }
                None => read.stopped = stopped.err(),
            }
        }
        let via = read.via();
        let stopped = read.stopped.map(|error| OpsError::at(&address, error));
        Ok(Readings {
            readings: readings(raw_readings(&rows, read.rows, read.partial)),
            via,
            stopped,
        })
    }

    /// `rows` read by batches at `at`, remembering a refusal of RPM.
    async fn batch_at(
        &self,
        at: &DeviceAddress,
        rows: &[BatchRow<usize>],
    ) -> Result<Batched<usize>, OpsError> {
        let client = &self.client;
        let rpm = !client.refused_rpm(at);
        let read = batch::read(client.low_level(), client.limits(), at, rows, rpm)
            .await
            .map_err(|error| OpsError::at(at, error))?;
        if read.refused_rpm {
            client.refuse_rpm(at);
        }
        Ok(read)
    }

    /// The properties `wanted` of `object`: one ReadPropertyMultiple, or
    /// ReadProperty each once the device has refused RPM, or when it failed
    /// this RPM as a whole (with an Error PDU, say), as
    /// [`read_many`](Self::read_many) reads a batch it fails. For
    /// [`Wanted::All`], by ReadProperty the object's Property_List names
    /// them. A property the device can't give is an error in its own row.
    /// A value's text names its object's Units and its state by the
    /// object's texts when the read holds them.
    ///
    /// # Errors
    ///
    /// A device that can't be placed, or stops answering ([`OpsError`]'s
    /// `unreachable()`): nothing read is kept, since the object's rows go
    /// together.
    pub async fn read_object(
        &self,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<ObjectRead, OpsError> {
        let (rows, via) = self.rows(object, wanted).await?;
        Ok(ObjectRead {
            object,
            readings: readings(rows.into_iter().map(|row| (object, None, row)).collect()),
            via,
        })
    }

    /// [`read_object`](Self::read_object)'s rows, undecoded.
    pub(crate) async fn rows(
        &self,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<(Vec<RawRow>, ReadVia), OpsError> {
        sent!(self, |at| self.rows_at(at, object, wanted).await)
    }

    /// [`rows`](Self::rows) at `at`, as the low-level client fails.
    pub(super) async fn rows_at(
        &self,
        at: &DeviceAddress,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<(Vec<RawRow>, ReadVia), Error> {
        let mut reader = self.client.reader(at, self.instance());
        let read = reader.properties(object, wanted).await;
        self.client.settle(&reader);
        match read {
            // An RPM the device failed as a whole, not by refusing the
            // service or for its size: read this object by ReadProperty
            // this time, as a bulk read reads such a batch. Not when the
            // client's own link failed it (#1833): ReadProperty would fail
            // the same way.
            Err(Halt::Device(error))
                if reader.rpm
                    && !unreachable(&error)
                    && !failed_locally(&error)
                    && rpm_failure(&error) == RpmFailure::Other =>
            {
                tracing::debug!(
                    "{object}: the RPM failed as a whole ({error}); reading one property at a time"
                );
                let mut once = self.client.reader(at, self.instance());
                once.rpm = false;
                once.properties(object, wanted).await.map_err(device_error)
            }
            read => read.map_err(device_error),
        }
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

    /// The device's whole Object_List, read as [`list_objects`](Self::list_objects)
    /// reads it, for a caller that needs every object or none, as an
    /// [`inventory`](Self::inventory) does: a list read element by element
    /// that fails at one element is [`OpsError::ObjectList`], which names
    /// the element and holds why it failed (a timeout stays one,
    /// [`OpsError::timed_out`]), where `list_objects` keeps the elements
    /// before it and says why as text.
    ///
    /// # Errors
    ///
    /// What `list_objects` fails with, and [`OpsError::ObjectList`].
    pub async fn list_objects_whole(&self) -> Result<Vec<ObjectIdentifier>, OpsError> {
        // Which element of a list read element by element failed, for the
        // error to name.
        let mut element;
        let read = sent!(self, |at| {
            element = None;
            let reader = self.client.reader(at, self.instance());
            match reader.object_list_until(|_, _| {}).await {
                Ok((list, None)) => Ok(list.objects),
                Ok((_, Some(stop))) => {
                    element = Some((stop.index, stop.total));
                    Err(stop.error)
                }
                Err(halt) => Err(device_error(halt)),
            }
        });
        match (read, element) {
            (Ok(objects), _) => Ok(objects),
            (Err(source), Some((index, total))) => Err(OpsError::ObjectList {
                index,
                total,
                source: Box::new(source),
            }),
            (Err(error), None) => Err(error),
        }
    }

    /// The device's [`DEVICE_PROPERTIES`]. A handle that doesn't know the
    /// instance reads it first ([`resolve_instance`](Self::resolve_instance)).
    /// The client notes which COV services its Protocol_Services_Supported
    /// lists ([`cov_support`](Self::cov_support)).
    pub async fn describe_device(&self) -> Result<DeviceSummary, OpsError> {
        let device = self.resolve_instance().await?;
        let object = ObjectIdentifier::new(ObjectType::DEVICE, device)?;
        let (rows, via) = self.rows(object, Wanted::List(&DEVICE_PROPERTIES)).await?;
        let services = rows
            .iter()
            .find(|(p, _)| *p == PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
            .and_then(|(_, value)| crate::watch::services(value.as_deref().ok()?));
        if let Some(services) = services {
            self.client.cov_declared(&self.address(), services);
        }
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
            let path = path_to(self.client.low_level(), at).await?;
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

    /// The station `at` names, and the device's turn there.
    pub(super) async fn turn_at(&self, at: &DeviceAddress) -> Result<(Station, Turn), Error> {
        let path = path_to(self.client.low_level(), at).await?;
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
            client.get_event_information(&path, after).await
        })
    }

    /// The device's GetAlarmSummary answer: the objects whose alarms it
    /// reports this way.
    pub async fn alarm_summary(&self) -> Result<GetAlarmSummaryAck, OpsError> {
        let client = self.client.low_level();
        sent!(self, |at| {
            let (path, _turn) = self.turn_at(at).await?;
            client.get_alarm_summary(&path).await
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
            client.get_enrollment_summary(&path, request).await
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
    /// a device that stops answering, the client's own link failing (a
    /// stopped transport, #1833), or `out_of_time` turning true, ends the
    /// reading before the next object. The [`Naming`] says whether the
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
                    // The device stopped answering, or the client's own
                    // link failed (#1833): every later read would fail the
                    // same way, so the reading ends here.
                    Err(error) if error.unreachable() || error.failed_locally() => {
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
/// out. The last row read only in part (`partial`) gives the properties it
/// read and no others.
fn raw_readings(
    asked: &[BatchRow<usize>],
    read: Vec<(usize, Result<Vec<RawRow>, Missing>)>,
    partial: bool,
) -> Vec<RawReading> {
    let mut out = Vec::new();
    let last = read.len().checked_sub(1);
    for (at, (row, (_, answer))) in asked.iter().zip(read).enumerate() {
        let partial = partial && Some(at) == last;
        let mut answer: Vec<Option<RawRow>> = answer
            .map(|rows| rows.into_iter().map(Some).collect())
            .unwrap_or_default();
        for &(property, index) in &row.wanted {
            let found = answer
                .iter_mut()
                .find(|slot| slot.as_ref().is_some_and(|(p, _)| *p == property))
                .and_then(Option::take);
            match found {
                Some((_, value)) => out.push((row.object, index, (property, value))),
                // Not read before the device stopped answering.
                None if partial => break,
                None => out.push((row.object, index, (property, Err(PropertyError::NoValue)))),
            }
        }
    }
    out
}
