//! [`PyDevice`]: the Python `Device` handle (#2561), one device as a
//! `BACnetClient` reaches it.
//!
//! The Python object keeps what it was named by ([`Named`]) and its tie to
//! the client ([`DeviceTie`]), never the high-level client itself: the
//! started client keeps the handle, each call finds it there under the
//! lease every call takes, and a stop drops it with the rest. So a
//! `Device` left alive never holds a stop back, a call on one after
//! `stop()` raises what any call raises then, and the first call after a
//! restart makes the handle again from what named it.

use std::sync::{Arc, Mutex};

use bacnet_ops::PropertyRef;
use bacnet_ops::client::Wanted;
use bacnet_ops::target::AddressStyle;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::sync::MutexExt;
use pyo3::prelude::*;

use super::errors::ops_err;
use super::points::{ReadInventory, cov_label, objects_arg, once_each, options};
use super::preview::PyWritePreview;
use super::priorities::{PyWritten, priority_arg};
use super::readings::{PyDeviceSummary, PyObjectList, PyReadings};
use super::specs::{Named, object_arg, property_arg, property_item, refuse_text, value_arg};
use super::value::PyValue;
use super::walk::PyPointIterator;
use super::{OpsClient, OpsDevice};
use crate::client::{DeviceTie, Held};
use crate::py_async::{attach, future_into_py};

/// What a handle last said of itself, for the getters once its client has
/// stopped.
#[derive(Debug, Clone)]
struct Seen {
    address: String,
    instance: Option<u32>,
    follows: bool,
}

impl Seen {
    fn of(device: &OpsDevice) -> Self {
        Self {
            address: device.address().styled(device.client().address_style()),
            instance: device.instance(),
            follows: device.follows(),
        }
    }
}

/// A `Device`'s state, shared with its calls' futures and its point
/// iterators.
pub(super) struct Core {
    tie: DeviceTie,
    named: Named,
    seen: Mutex<Seen>,
}

impl Core {
    /// The handle, held for one call, made again from what named it when
    /// the client now started has none.
    async fn hold(&self) -> PyResult<(Held<'_>, AddressStyle)> {
        let held = self.tie.hold(|client| make(client, &self.named)).await?;
        let style = held.device.client().address_style();
        self.see(Seen::of(&held.device));
        Ok((held, style))
    }

    /// Run `call` on the handle, held with the client's lease for it.
    pub(super) async fn run<F, R>(
        &self,
        call: impl FnOnce(OpsDevice, AddressStyle) -> F,
    ) -> PyResult<R>
    where
        F: Future<Output = PyResult<R>>,
    {
        // Boxed, as the deepest async chain in this crate runs through
        // here: a station lookup down to a link's send. Unboxed, the
        // crate needed rustc's whole default recursion limit, 128, when
        // this was boxed (#2617), and boxed 109; the chain below has grown
        // since, and boxed the crate now needs 115 (114 fails), as it did
        // before this hold moved into `run`. Boxing it as a trait object
        // needs the same. One allocation per Python Device call, off the
        // Rust request path.
        let (held, style) = Box::pin(self.hold()).await?;
        // The call's own clone goes before the lease does: `held`
        // outlives the call's future.
        let result = call(held.device.clone(), style).await;
        self.see(Seen::of(&held.device));
        result
    }

    /// Its tie to the client.
    pub(super) fn tie(&self) -> &DeviceTie {
        &self.tie
    }

    fn see(&self, seen: Seen) {
        *self.seen.locked() = seen;
    }

    /// What the handle says of itself now, or last said.
    fn seen(&self) -> Seen {
        if let Some(seen) = self.tie.peek(Seen::of) {
            self.see(seen);
        }
        let seen = self.seen.locked();
        seen.clone()
    }
}

/// The handle `named` names on `client`.
async fn make(client: OpsClient, named: &Named) -> PyResult<OpsDevice> {
    super::device(&client, named).await
}

/// One device, as a `BACnetClient` reaches it: what `await
/// client.device(target)` returns.
///
/// Made from an address (`"192.168.1.10"`, `"mstp:5"`, `"2:0x05"`, a
/// `discover()` row) it is pinned there. Made from an instance (`1234`,
/// `"2:1234"`) it follows the instance: when the device stops answering at
/// its address and one other address claims the instance, the handle
/// moves there. Its calls go one at a time to the device, with the
/// client's other high-level calls to it.
///
/// A handle belongs to its client. After the client's `stop()`, a call
/// raises RuntimeError, as any call on a stopped client does, and
/// `address` and `instance` say what was last known; after the client
/// starts again, the next call makes the handle again from what it was
/// named by (an instance is looked up afresh), and it works as before.
/// Refuses `copy` and `pickle`.
#[pyclass(name = "Device", module = "rusty_bacnet", frozen)]
pub(crate) struct PyDevice {
    core: Arc<Core>,
}

impl PyDevice {
    /// The handle for `named` on the client `tie` belongs to, made now:
    /// what `BACnetClient.device` awaits.
    pub(crate) async fn open(tie: DeviceTie, named: Named) -> PyResult<Self> {
        let core = Arc::new(Core {
            tie,
            named,
            seen: Mutex::new(Seen {
                address: String::new(),
                instance: None,
                follows: false,
            }),
        });
        core.hold().await?;
        Ok(Self { core })
    }

    /// Run `call` on the handle, held for it, as a Python awaitable.
    fn call<'py, F, R>(
        &self,
        py: Python<'py>,
        call: impl FnOnce(OpsDevice, AddressStyle) -> F + Send + 'static,
    ) -> PyResult<Bound<'py, PyAny>>
    where
        F: Future<Output = PyResult<R>> + Send,
        R: for<'a> IntoPyObject<'a> + Send + 'static,
    {
        let core = Arc::clone(&self.core);
        future_into_py(py, async move { core.run(call).await })
    }
}

/// `property`'s parts, as [`PyValue::new`] takes them.
fn value_of(property: PropertyRef, value: &bacnet_ops::Value) -> PyValue {
    PyValue::new(
        property.object.object_type(),
        property.property,
        property.index,
        value,
    )
}

#[pymethods]
impl PyDevice {
    /// Where the device answers now, as text `BACnetClient.device` reads
    /// back: a pinned handle's address, or where a following handle last
    /// reached its instance.
    #[getter]
    fn address(&self) -> String {
        self.core.seen().address
    }

    /// The device instance, when known: from discovery, the instance the
    /// handle follows, or an earlier `summary()` or `resolve_instance()`.
    /// None for a handle made from an address nothing has named yet.
    #[getter]
    fn instance(&self) -> Option<u32> {
        self.core.seen().instance
    }

    /// True when the handle follows its instance, False when it is pinned
    /// to its address.
    #[getter]
    fn follows(&self) -> bool {
        self.core.seen().follows
    }

    /// What is known of the device's COV support, with nothing sent:
    /// `"unknown"`, `"not-offered"` or `"offered"` (from the services
    /// `summary()` read), `"accepted"` or `"refused"` (from a subscription
    /// a watch made). `"unknown"` while the client is stopped.
    fn cov_support(&self) -> &'static str {
        let support = self.core.tie.peek(OpsDevice::cov_support);
        cov_label(support.unwrap_or_default())
    }

    /// Read one property: one ReadProperty, as a `Value`.
    ///
    /// `object` is an `ObjectIdentifier` or text (`"ai:1"`,
    /// `"analog-input:1"`); `property` a `PropertyIdentifier` or text
    /// (`"pv"`, `"present-value"`, `"priority-array[8]"`), Present_Value
    /// when left out; `index` an array element (0 is an array's size).
    /// With no `property`, `object` may be both as text: `"ai:1 pv"`. The
    /// text names no units and no state: `read_many` with the object's
    /// units or state texts in the same call names them.
    ///
    /// A wrong type raises TypeError and text the grammar refuses
    /// ValueError, before the Future is made. On await: the device's
    /// refusal raises BacnetProtocolError, BacnetRejectError or
    /// BacnetAbortError; no answer BacnetTimeoutError; a following handle
    /// whose device can't be placed any more BacnetLookupError.
    #[pyo3(signature = (object, property=None, *, index=None))]
    fn read<'py>(
        &self,
        py: Python<'py>,
        object: &Bound<'py, PyAny>,
        property: Option<&Bound<'py, PyAny>>,
        index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wanted = property_arg(object, property, index)?;
        self.call(py, move |device, style| async move {
            let value = device.read(wanted).await;
            let value = value.map_err(|error| ops_err(error, style))?;
            Ok(value_of(wanted, &value))
        })
    }

    /// Read many properties of any of the device's objects, in batches of
    /// ReadPropertyMultiple (ReadProperty each once the device has refused
    /// that), as `Readings`, in the order asked for.
    ///
    /// Each item of `properties` is text (`"ai:1 pv"`, `"ai:1"`), an
    /// `ObjectIdentifier` (its Present_Value), or an `(object, property)`
    /// or `(object, property, index)` tuple of the forms `read` takes. A
    /// value's text names its object's units, and its state by the
    /// object's texts, when the same call reads them.
    ///
    /// A property the device refuses, or one of an object it doesn't have,
    /// never raises: its `Reading.error` is set and the rest stand. A
    /// device that stops answering part way ends the read with what was
    /// read, and `Readings.stopped` says so; so does a later request the
    /// client's own link fails (a route lost, a send refused). A read
    /// raises only when nothing could be read: `properties` as one str,
    /// bytes or bytearray, or a bad item, TypeError or ValueError (ALL,
    /// REQUIRED and OPTIONAL among them: `read_object` reads those) before
    /// the Future is made; on await a device that can't be placed
    /// BacnetLookupError, and a link that fails the first request what a
    /// single read raises for it.
    fn read_many<'py>(
        &self,
        py: Python<'py>,
        properties: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        refuse_text(
            properties,
            "properties",
            "['ai:1', 'ao:2 priority-array[8]']",
        )?;
        let wanted = properties
            .try_iter()?
            .map(|item| property_item(&item?))
            .collect::<PyResult<Vec<_>>>()?;
        for property in &wanted {
            many_refused(property.property)?;
        }
        self.call(py, move |device, style| async move {
            let read = device.read_many(wanted).await;
            let read = read.map_err(|error| ops_err(error, style))?;
            attach(|py| PyReadings::new(py, read, style))
        })
    }

    /// Read one object's properties in one ReadPropertyMultiple (or by
    /// ReadProperty each once the device has refused that), as `Readings`:
    /// every property when `properties` is left out, else those listed
    /// (`PropertyIdentifier`s or text, whole properties only; one str,
    /// bytes or bytearray raises TypeError before the Future is made). A
    /// property the device refused has its `Reading.error` set. A device
    /// that stops answering raises BacnetTimeoutError and keeps nothing,
    /// since an object's rows go together: `stopped` is always None here.
    #[pyo3(signature = (object, properties=None))]
    fn read_object<'py>(
        &self,
        py: Python<'py>,
        object: &Bound<'py, PyAny>,
        properties: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let object_id = object_arg(object)?;
        let listed = properties
            .filter(|properties| !properties.is_none())
            .map(|properties| {
                refuse_text(properties, "properties", "['pv', 'units']")?;
                properties
                    .try_iter()?
                    .map(|item| {
                        let wanted = property_arg(object, Some(&item?), None)?;
                        match wanted.index {
                            None => Ok(wanted.property),
                            Some(_) => Err(pyo3::exceptions::PyValueError::new_err(
                                "read_object reads whole properties: read an element with \
                                 read or read_many",
                            )),
                        }
                    })
                    .collect::<PyResult<Vec<_>>>()
            })
            .transpose()?;
        self.call(py, move |device, style| async move {
            let wanted = listed.as_deref().map_or(Wanted::All, Wanted::List);
            let read = device.read_object(object_id, wanted).await;
            let read = read.map_err(|error| ops_err(error, style))?;
            attach(|py| PyReadings::of_object(py, &read.readings, read.via))
        })
    }

    /// Write one property: one WriteProperty, acknowledged, as `Written`.
    ///
    /// `object`, `property` and `index` are as `read` takes them. `value`
    /// is text in the value grammar, read for the property's datatype
    /// (`"72.5"`, `"active"`, `"72.5@8"` with a priority), an int or a
    /// float (read as its text), or a typed `PropertyValue`, sent as it
    /// is. The priority is `priority`, or the one the text names, or both
    /// when they agree. A property the standard makes commandable (an
    /// output's Present_Value) goes at the priority given, or at 16; one
    /// that may be (a value object's) at the priority given, or with none;
    /// any other with none.
    ///
    /// Raised with nothing sent: TypeError or ValueError before the Future
    /// is made for a wrong type, bad text or a priority outside 1 to 16;
    /// on await ValueError for a value the property's datatype refuses,
    /// two different priorities, a priority on a property that isn't
    /// commandable, NULL to a commandable Present_Value (use `relinquish`)
    /// and any write to Priority_Array; and BacnetWriteDeniedError for a
    /// priority the client's `write_policy` refuses (1, 2, 5 and 6 unless
    /// allowed). Then the device's refusal raises the usual protocol,
    /// reject or abort error, and no answer BacnetTimeoutError: the write
    /// may or may not have been applied, and is never sent again.
    #[pyo3(signature = (object, property, value, *, priority=None, index=None))]
    fn write<'py>(
        &self,
        py: Python<'py>,
        object: &Bound<'py, PyAny>,
        property: &Bound<'py, PyAny>,
        value: &Bound<'py, PyAny>,
        priority: Option<&Bound<'py, PyAny>>,
        index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wanted = property_arg(object, Some(property), index)?;
        let value = value_arg(value)?;
        let priority = optional_priority(priority)?;
        self.call(py, move |device, style| async move {
            let written = device.write(wanted, value, priority).await;
            let written = written.map_err(|error| ops_err(error, style))?;
            attach(|py| PyWritten::new(py, &written, style))
        })
    }

    /// What `write` of the same arguments would do, read first and sent
    /// nowhere, as a `WritePreview`: the slot it takes and who holds it,
    /// whether a stronger slot keeps Present_Value where it is, and what
    /// Present_Value becomes. One request: a read of the object's
    /// priorities for a Present_Value that may be commandable, or of the
    /// property for any other. It raises what `write` raises before
    /// sending, except for the write policy, which `WritePreview.denied`
    /// reports instead.
    #[pyo3(signature = (object, property, value, *, priority=None, index=None))]
    fn preview_write<'py>(
        &self,
        py: Python<'py>,
        object: &Bound<'py, PyAny>,
        property: &Bound<'py, PyAny>,
        value: &Bound<'py, PyAny>,
        priority: Option<&Bound<'py, PyAny>>,
        index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wanted = property_arg(object, Some(property), index)?;
        let value = value_arg(value)?;
        let priority = optional_priority(priority)?;
        self.call(py, move |device, style| async move {
            let preview = device.preview_write(wanted, value, priority).await;
            let preview = preview.map_err(|error| ops_err(error, style))?;
            attach(|py| PyWritePreview::new(py, &preview, style))
        })
    }

    /// Release a command: NULL written to `object`'s Present_Value at
    /// `priority`, then the object's priorities as read just after, as a
    /// `PriorityView`: one WriteProperty, then one ReadPropertyMultiple
    /// (a ReadProperty of each of the three properties when the device
    /// refuses that). `object` is an `ObjectIdentifier` or text
    /// (`"ao:1"`); `priority` is required, an int from 1 to 16: a bool, a
    /// float or anything else raises TypeError and another int ValueError,
    /// before the Future is made.
    ///
    /// On await, with nothing sent: an object type with no Priority_Array
    /// raises ValueError, and a priority the client's `write_policy`
    /// refuses (1, 2, 5 and 6 unless allowed) BacnetWriteDeniedError. Then
    /// the device's refusal of the write raises the usual protocol, reject
    /// or abort error, and no answer BacnetTimeoutError. A relinquish the
    /// device acknowledged whose read after it failed raises
    /// BacnetReadBackError, and one whose read says the object has no
    /// Priority_Array BacnetNoPriorityArrayError; both say what was
    /// written (`written`), which needs no second send.
    #[pyo3(signature = (object, priority))]
    fn relinquish<'py>(
        &self,
        py: Python<'py>,
        object: &Bound<'py, PyAny>,
        priority: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let object = object_arg(object)?;
        let priority = priority_arg(priority)?;
        self.call(py, move |device, _| async move {
            super::relinquish(&device, object, priority).await
        })
    }

    /// Read `object`'s commands: its Priority_Array, Relinquish_Default
    /// and Present_Value in one request, as a `PriorityView`. `object` is
    /// an `ObjectIdentifier` or text (`"ao:1"`). An object type with no
    /// Priority_Array raises ValueError on await with nothing sent; a
    /// device that says the object has none BacnetNoPriorityArrayError;
    /// no answer BacnetTimeoutError.
    #[pyo3(signature = (object))]
    fn priority_array<'py>(
        &self,
        py: Python<'py>,
        object: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let object = object_arg(object)?;
        self.call(py, move |device, _| async move {
            super::priority_array(&device, object).await
        })
    }

    /// The device's Object_List, as an `ObjectList`: whole, or an element
    /// at a time when the device can't send it in one answer.
    fn objects<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, move |device, style| async move {
            let list = device.list_objects().await;
            let list = list.map_err(|error| ops_err(error, style))?;
            attach(|py| PyObjectList::new(py, &list))
        })
    }

    /// The device's summary, as a `DeviceSummary`: its name, vendor,
    /// model, firmware, protocol revision and what it supports, in one
    /// request. A handle that doesn't know its instance reads that first.
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, move |device, style| async move {
            let summary = device.describe_device().await;
            let summary = summary.map_err(|error| ops_err(error, style))?;
            attach(|py| PyDeviceSummary::new(py, &summary))
        })
    }

    /// The device's objects as point records, as an `Inventory`: the
    /// device's summary, its Object_List, then the objects of `types` (all
    /// by default; `ObjectType`s or text such as `"ai"`) in batches of
    /// ReadPropertyMultiple, up to 16 objects or 32 properties a request
    /// (ReadProperty each once the device has refused that). Each record
    /// holds the object's name, description, units or state texts and
    /// whether it takes commands; `values` adds its Present_Value,
    /// Status_Flags, Event_State, Reliability and priority in control,
    /// `settings` its COV_Increment, range, Relinquish_Default and
    /// Polarity, `details` its Device_Type and Profile_Name.
    ///
    /// The whole read is one call: it holds the client throughout, so a
    /// `stop()` waits for it, and nothing comes back until every object is
    /// read. For a large device, `points` hands the same records out a
    /// batch at a time.
    ///
    /// A property the device can't give is a problem on its record, never
    /// raised. A device that goes silent while its objects are read ends
    /// the read with the records read so far: `Inventory.stopped` says why
    /// and `Inventory.unread` lists the rest. Raised: a bad argument
    /// TypeError or ValueError, before the Future is made; on await a
    /// device that can't be placed BacnetLookupError, and one that doesn't
    /// answer the summary or the Object_List, or refuses the list, what a
    /// single read raises (a list read element by element that fails at
    /// one element names it).
    #[pyo3(signature = (*, types=None, values=false, settings=false, details=false))]
    fn inventory<'py>(
        &self,
        py: Python<'py>,
        types: Option<&Bound<'py, PyAny>>,
        values: bool,
        settings: bool,
        details: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = options(types, values, settings, details)?;
        self.call(py, move |device, style| async move {
            let read = device.inventory(&options).await;
            let read = read.map_err(|error| ops_err(error, style))?;
            // Made an `Inventory` when it reaches Python, on the bridge's
            // blocking thread, once the lease has gone.
            Ok(ReadInventory(Box::new(read), style))
        })
    }

    /// The device's points as an async iterator, read as they are
    /// iterated: a `PointIterator` of `Point` records, as `inventory`
    /// reads them, for a large device or a screen that fills as it goes.
    /// The await reads the Object_List (whole, or element by element),
    /// keeping the objects of `types`, each once; or, with `objects`
    /// (`ObjectIdentifier`s or text), takes those of `types`, each once, in
    /// order, and reads nothing. Iterating reads the next batch of objects
    /// whenever the last batch's points are out: one ReadPropertyMultiple
    /// while the device takes them, so a large device's first points come
    /// at once and an `async for` broken off reads no more. Each batch
    /// holds the client for its read alone; `inventory` reads the whole
    /// device in one call.
    ///
    /// A device that stops answering raises from `__anext__`, once the
    /// points read before it are out; iterating again tries the first
    /// object not read again. A cancelled `__anext__` stops its request,
    /// and the next reads that batch again, so no point is lost. Raised: a
    /// bad argument TypeError or ValueError, before the Future is made; on
    /// await what `inventory` raises for the Object_List.
    #[pyo3(signature = (objects=None, *, types=None, values=false, settings=false, details=false))]
    fn points<'py>(
        &self,
        py: Python<'py>,
        objects: Option<&Bound<'py, PyAny>>,
        types: Option<&Bound<'py, PyAny>>,
        values: bool,
        settings: bool,
        details: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = options(types, values, settings, details)?;
        let named = objects
            .filter(|objects| !objects.is_none())
            .map(objects_arg)
            .transpose()?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let listed = match named {
                // The lease, as every call takes it: a stopped client
                // raises here, with nothing sent.
                Some(named) => core.run(|_, _| async move { Ok(named) }).await?,
                None => {
                    let list = |device: OpsDevice, style| async move {
                        let list = device.list_objects_whole().await;
                        list.map_err(|error| ops_err(error, style))
                    };
                    core.run(list).await?
                }
            };
            let objects = once_each(listed, |object| options.reads(object.object_type()));
            Ok(PyPointIterator::new(core, objects, options))
        })
    }

    /// Watch `points` of the device, as a `Watch`: an async iterator of
    /// `WatchUpdate`s, each point's latest value and how its watch goes.
    ///
    /// Each item of `points` is what `read_many` takes: text (`"ai:1"`,
    /// `"ao:2 priority-array[8]"`), an `ObjectIdentifier` (its
    /// Present_Value), or an `(object, property)` or `(object, property,
    /// index)` tuple; each point once. A Present_Value is watched by
    /// SubscribeCOV on its object, any other property or element by
    /// SubscribeCOVProperty. Each point is read once, then subscribed for
    /// `lifetime` seconds at a time (1 to 28800), renewed before each
    /// lifetime ends; a point the device won't subscribe, or every point
    /// with `subscribe=False`, is polled, with the device's other polled
    /// points, every `poll_interval` seconds (1 to 3600). A keyword left
    /// None takes the client's (`watch_lifetime`, `watch_poll_interval`,
    /// `watch_subscribe`); watches with equal settings share their
    /// subscriptions.
    ///
    /// A handle made from an instance (`follows`) watches its device
    /// wherever it answers: once the watch's own reads find the device at
    /// another address and its old one no longer answering as it, the
    /// points move there, each update saying so (`moved`, a `WatchMove`)
    /// and where it is watched (`address`); while another device answers
    /// at its address the points are `"unreachable"` (`why.kind ==
    /// "replaced"`), never taken for it. A handle made from an address
    /// watches that address: a device that moves away is reported
    /// (`"gone"`, `why.kind == "moved"`), another device there too
    /// (`"replaced"`). An announcement alone moves nothing.
    ///
    /// The watch runs on its own until it ends: `close()` (or leaving
    /// `async with`), the object going away, or the client's `stop()`.
    /// Its iterator then raises StopAsyncIteration, and its subscriptions
    /// are cancelled, waiting up to 5 s for the device's answers. Raised: a
    /// bad argument TypeError or ValueError (one str, no points, more than
    /// 4096, ALL, REQUIRED or OPTIONAL, a lifetime outside 1 to 28800, a
    /// poll_interval outside 1 to 3600, a subscribe that isn't a bool)
    /// before the Future is made; on await what any call raises on a
    /// stopped client, BacnetLookupError for a following handle whose
    /// device can't be placed, and RuntimeError when the client already
    /// holds watches of 256 different settings. Nothing is sent until the
    /// watch starts.
    #[pyo3(
        signature = (points, *, lifetime=None, poll_interval=None, subscribe=None),
        text_signature = "($self, points, *, lifetime=None, poll_interval=None, subscribe=None)"
    )]
    fn watch<'py>(
        &self,
        py: Python<'py>,
        points: &Bound<'py, PyAny>,
        lifetime: Option<&Bound<'py, PyAny>>,
        poll_interval: Option<&Bound<'py, PyAny>>,
        subscribe: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wanted = super::watch::points_arg(points)?;
        let overrides = super::watch::Overrides::new(lifetime, poll_interval, subscribe)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, super::watch::open(core, wanted, overrides))
    }

    /// The device instance: as known (`instance`), or from one read of the
    /// Device object by the wildcard instance, remembered afterwards.
    fn resolve_instance<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, move |device, style| async move {
            let instance = device.resolve_instance().await;
            instance.map_err(|error| ops_err(error, style))
        })
    }

    /// For a pinned handle whose instance is known: the one other address
    /// its instance is heard at now, while its own has gone quiet, as text
    /// `BACnetClient.device` reads back; else None. Always None for a
    /// following handle, which moves itself. Sends nothing.
    fn moved_to<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, move |device, style| async move {
            Ok(device.moved_to().await.map(|to| to.styled(style)))
        })
    }

    fn __repr__(&self) -> String {
        let seen = self.core.seen();
        let instance = seen
            .instance
            .map_or_else(|| "None".to_owned(), |instance| instance.to_string());
        let pin = if seen.follows { "follows" } else { "pinned" };
        format!("Device({}, instance={instance}, {pin})", seen.address)
    }

    /// Refuses `copy` and `pickle`: a handle on a live client. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A `priority=` keyword: None, or what [`priority_arg`] takes.
fn optional_priority(
    priority: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<bacnet_ops::Priority>> {
    priority
        .filter(|priority| !priority.is_none())
        .map(priority_arg)
        .transpose()
}

/// Refuse a property that stands for many, when the call is made, as the
/// read itself would on await.
pub(super) fn many_refused(property: PropertyIdentifier) -> PyResult<()> {
    let many = [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ];
    if many.contains(&property) {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "'{}' names many properties; read every property of an object with read_object",
            bacnet_ops::label::kebab(property)
        )));
    }
    Ok(())
}
