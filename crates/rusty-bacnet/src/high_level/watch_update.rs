//! [`PyWatchUpdate`], [`PyWatchReason`] and [`PyWatchMove`]: what a
//! `Watch` hands out (#2561), one `bacnet-ops` watch [`Update`] decoded for
//! Python: the point, its latest value, how its watch goes, with the reason
//! when it isn't subscribed, and where a followed device moved (#2795).

use bacnet_ops::format::{Floats, StateTexts, ValueFormat};
use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::target::AddressStyle;
use bacnet_ops::watch::{Mark, Moved, State, Update, Via, Why};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, StatusFlags};
use pyo3::prelude::*;
use pyo3::types::PyString;

use super::points::cov_label;
use super::readings::row_error;
use super::value::PyValue;
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

#[cfg(test)]
#[path = "watch_update_tests.rs"]
mod tests;

/// `state` by name, with its reason, if it carries one.
pub(super) fn state_name(state: &State) -> (&'static str, Option<&Why>) {
    match state {
        State::Starting => ("starting", None),
        State::Subscribed => ("subscribed", None),
        State::Polled { why } => ("polled", Some(why)),
        State::Unreachable { why } => ("unreachable", Some(why)),
        State::Gone { why } => ("gone", Some(why)),
        _ => ("other", None),
    }
}

/// `flags`, an object of `object_type`'s Status_Flags, as the `Value` a
/// read of them gives (an inventory's `Point.status_flags`): the hub keeps
/// them decoded, so they are encoded again in their four-bit wire form.
fn status_flags_value(object_type: ObjectType, flags: StatusFlags) -> bacnet_ops::Value {
    let mut raw = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_bit_string(&mut raw, 4, &[flags.bits() << 4]);
    let texts = StateTexts::default();
    let format = ValueFormat::new(object_type, &texts, Floats::Exact);
    let property = PropertyIdentifier::STATUS_FLAGS;
    bacnet_ops::Value::new(&format, property, None, raw.to_vec(), None)
}

/// How `via`'s values came, by name.
fn via_name(via: Via) -> &'static str {
    match via {
        Via::First => "read",
        Via::Notification => "cov",
        Via::Poll => "poll",
    }
}

/// One change of a watched point, as a `Watch` hands it out: the point,
/// its latest value, and how its watch goes. Frozen; refuses `copy` and
/// `pickle`, as the other read results do, and its `Value` copies and
/// pickles.
#[pyclass(name = "WatchUpdate", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWatchUpdate {
    address: String,
    instance: Option<u32>,
    moved: Option<Py<PyWatchMove>>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    state: &'static str,
    why: Option<Py<PyWatchReason>>,
    value: Option<Py<PyValue>>,
    error: Option<Py<PyAny>>,
    status_flags: Option<Py<PyValue>>,
    via: Option<&'static str>,
    timestamp: Option<f64>,
    cov_support: &'static str,
    /// Which of its watch's points it was taken for, so one put back after
    /// its point's removal is dropped.
    mark: Mark,
}

impl PyWatchUpdate {
    /// `update`, its addresses shown in `style`.
    pub(super) fn new(py: Python<'_>, update: &Update, style: AddressStyle) -> PyResult<Self> {
        let object = update.point.object;
        let (property, index) = update.point.property();
        let address = update.at.styled(style);
        let moved = update
            .moved
            .as_ref()
            .map(|moved| Py::new(py, PyWatchMove::new(moved, style)))
            .transpose()?;
        let (state, why) = state_name(&update.state);
        let why = why
            .map(|why| Py::new(py, PyWatchReason::new(why, &address, style)))
            .transpose()?;
        let value = update
            .value
            .as_ref()
            .map(|value| {
                Py::new(
                    py,
                    PyValue::new(object.object_type(), property, index, value),
                )
            })
            .transpose()?;
        let status_flags = update
            .status_flags
            .map(|flags| {
                let value = status_flags_value(object.object_type(), flags);
                let property = PropertyIdentifier::STATUS_FLAGS;
                Py::new(
                    py,
                    PyValue::new(object.object_type(), property, None, &value),
                )
            })
            .transpose()?;
        Ok(Self {
            address,
            instance: update.instance,
            moved,
            object,
            property,
            index,
            state,
            why,
            value,
            error: update.error.as_ref().map(|error| row_error(py, error)),
            status_flags,
            via: update.via.map(via_name),
            timestamp: update.updated.map(crate::frames::posix_seconds),
            cov_support: cov_label(update.cov),
            mark: update.mark(),
        })
    }

    /// Which of its watch's points it was taken for.
    pub(super) fn mark(&self) -> Mark {
        self.mark
    }
}

#[pymethods]
impl PyWatchUpdate {
    /// Where the point is watched now, as text `BACnetClient.device`
    /// reads back: the watch's address, or, for a watch that follows its
    /// device, where the device answers now.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The device's instance, when known: the one a following watch
    /// follows, else the one the device at the address named, or the
    /// discovery table's.
    #[getter]
    fn instance(&self) -> Option<u32> {
        self.instance
    }

    /// For a watch that follows its device: the latest move the watch took,
    /// a `WatchMove` (it watches the point at `moved.to`, which is
    /// `address`, since `moved.timestamp`); None before any move, and
    /// always for a watch of one address. A consumer that falls behind sees
    /// the latest move only: compare its `count` to tell a new one. The
    /// update a move makes is `"starting"`, with the value read before;
    /// values from the new address follow.
    #[getter]
    fn moved(&self, py: Python<'_>) -> Option<Py<PyWatchMove>> {
        self.moved.as_ref().map(|moved| moved.clone_ref(py))
    }

    /// The object.
    #[getter]
    fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// The property whose value `value` is: Present_Value for a point named
    /// by its object alone.
    #[getter]
    fn property_id(&self) -> PyPropertyIdentifier {
        PyPropertyIdentifier {
            inner: self.property,
        }
    }

    /// The array element watched, None for the whole property.
    #[getter]
    fn array_index(&self) -> Option<u32> {
        self.index
    }

    /// How the point's watch goes: `"starting"` (its first read and
    /// subscription haven't finished), `"subscribed"` (values come as COV
    /// notifications), `"polled"` (read with the device's other polled
    /// points, `why` says why), `"unreachable"` (the device didn't answer
    /// the last request; polled again after a backoff) or `"gone"` (given
    /// up: nothing more is sent for it and nothing its address sends is
    /// taken as its own; `why` says whether the device moved or another
    /// device took its address). `"other"` for a state this version
    /// doesn't name.
    #[getter]
    fn state(&self) -> &'static str {
        self.state
    }

    /// Why the point isn't subscribed, a `WatchReason`; None while it is
    /// subscribed or starting.
    #[getter]
    fn why(&self, py: Python<'_>) -> Option<Py<PyWatchReason>> {
        self.why.as_ref().map(|why| why.clone_ref(py))
    }

    /// The value as last read or notified, a `Value` whose text names the
    /// object's units or state; None before any. With `error` set too, it
    /// is a value read earlier while the latest read failed.
    #[getter]
    fn value(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value.as_ref().map(|value| value.clone_ref(py))
    }

    /// Why the latest read gave no value, as the exception a typed read of
    /// the property would raise, never raised here: the device's error
    /// for the property or the object, or a `BacnetError` for an answer
    /// that left it out. None once a value was read or notified since.
    #[getter]
    fn error(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.error.as_ref().map(|error| error.clone_ref(py))
    }

    /// The object's Status_Flags as last read or notified with the value,
    /// a `Value` as an inventory's `Point.status_flags` is: its `value` a
    /// `PropertyValue` whose bit string names the set flags; None before
    /// any.
    #[getter]
    fn status_flags(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.status_flags.as_ref().map(|flags| flags.clone_ref(py))
    }

    /// How the value came: `"read"` (the watch's first read, or the read
    /// before a retried subscription), `"cov"` (a COV notification) or
    /// `"poll"`; None before any value.
    #[getter]
    fn via(&self) -> Option<&'static str> {
        self.via
    }

    /// When the value came, as POSIX seconds by the wall clock, as
    /// `time.time()`; None before any.
    #[getter]
    fn timestamp(&self) -> Option<f64> {
        self.timestamp
    }

    /// What is known of the point's COV subscription: `"unknown"`,
    /// `"not-offered"` or `"offered"` (from the device's services, before
    /// it was asked), `"accepted"` or `"refused"`.
    #[getter]
    fn cov_support(&self) -> &'static str {
        self.cov_support
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let mut point = format!("{}, {}", object_label(self.object), kebab(self.property));
        if let Some(index) = self.index {
            point.push_str(&format!("[{index}]"));
        }
        let value = match &self.value {
            Some(value) => PyString::new(py, value.get().text()).repr()?.to_string(),
            None => "None".to_owned(),
        };
        let via = self.via.unwrap_or("None");
        Ok(format!(
            "WatchUpdate({point}, {}, value={value}, via={via})",
            self.state
        ))
    }

    /// Refuses `copy` and `pickle`. See [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Why a watched point isn't subscribed: polled, unreachable or given up.
/// `kind` names it; the fields that kind carries are set, the others None;
/// `str()` says it in a line. Frozen; refuses `copy` and `pickle`.
#[pyclass(name = "WatchReason", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWatchReason {
    kind: &'static str,
    error: Option<String>,
    to: Option<String>,
    watched: Option<u32>,
    found: Option<u32>,
    sender: Option<u32>,
    text: String,
}

impl PyWatchReason {
    /// `why`, for a point at `address`, an address in it shown in `style`.
    pub(super) fn new(why: &Why, address: &str, style: AddressStyle) -> Self {
        let reason = |kind, error: Option<&String>, text: String| Self {
            kind,
            error: error.cloned(),
            to: None,
            watched: None,
            found: None,
            sender: None,
            text,
        };
        match why {
            Why::Refused(error) => reason(
                "refused",
                Some(error),
                format!("the device refused the subscription: {error}"),
            ),
            Why::PollOnly => reason("poll-only", None, "the watch polls every point".to_owned()),
            Why::NoAnswer(error) => reason(
                "no-answer",
                Some(error),
                format!("the device didn't answer the read or the subscription: {error}"),
            ),
            Why::Full(error) => reason(
                "full",
                Some(error),
                format!("the device's subscription table was full: {error}"),
            ),
            Why::Lapsed(Some(error)) => {
                reason("lapsed", Some(error), format!("a renewal failed: {error}"))
            }
            Why::Lapsed(None) => reason("lapsed", None, "the renewals stopped".to_owned()),
            Why::Missing => reason(
                "missing",
                None,
                "no router to the device's network is known".to_owned(),
            ),
            Why::Moved { to } => {
                let to = to.styled(style);
                Self {
                    text: format!("the device answers at {to} now: watch the point there"),
                    to: Some(to),
                    ..reason("moved", None, String::new())
                }
            }
            Why::Replaced { watched, found } => Self {
                watched: Some(*watched),
                found: Some(*found),
                ..reason(
                    "replaced",
                    None,
                    format!("device {found} answers at {address} now, not device {watched}"),
                )
            },
            Why::Unconfirmed => reason(
                "unconfirmed",
                None,
                format!(
                    "the device at {address} refuses the read that names it, so the device \
                     can't be confirmed there; watch it by its address"
                ),
            ),
            Why::ForeignSender { sender } => Self {
                sender: Some(*sender),
                ..reason(
                    "foreign-sender",
                    None,
                    format!(
                        "its notifications name device {sender}, which can't be told from \
                         another device at {address}"
                    ),
                )
            },
            other => reason("other", None, format!("{other:?}")),
        }
    }
}

#[pymethods]
impl PyWatchReason {
    /// The reason: `"refused"` (the device refused the subscription; not
    /// asked again), `"poll-only"` (the watch polls every point,
    /// `subscribe=False`), `"no-answer"` (the first read or the
    /// subscription went unanswered), `"full"` (the device's subscription table was
    /// full) or `"lapsed"` (a renewal failed), each subscribed again after
    /// a backoff; `"missing"` (no router to a remote device's network is
    /// known); `"moved"` (the device answers at `to` now) or `"replaced"`
    /// (device `found` answers at the address now, not `watched`), either
    /// of which gives the point up; `"foreign-sender"` (its notifications
    /// name device `sender`, which can't be told from another device
    /// there, so it is polled); `"unconfirmed"` (a following watch's device
    /// refuses the read that names it at its address, so nothing it answers is taken
    /// there: watch it by its address). `"other"` for a reason this version
    /// doesn't name.
    #[getter]
    fn kind(&self) -> &'static str {
        self.kind
    }

    /// The device's error, or the client's, as text, for `"refused"`,
    /// `"no-answer"`, `"full"` and `"lapsed"`; else None.
    #[getter]
    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// For `"moved"`: where the device answers now, as text
    /// `BACnetClient.device` reads back; else None.
    #[getter]
    fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }

    /// For `"replaced"`: the instance an earlier answer named for the
    /// point; else None.
    #[getter]
    fn watched(&self) -> Option<u32> {
        self.watched
    }

    /// For `"replaced"`: the instance the device at the address answers
    /// for now; else None.
    #[getter]
    fn found(&self) -> Option<u32> {
        self.found
    }

    /// For `"foreign-sender"`: the instance the notifications name; else
    /// None.
    #[getter]
    fn sender(&self) -> Option<u32> {
        self.sender
    }

    fn __str__(&self) -> &str {
        &self.text
    }

    fn __repr__(&self) -> String {
        format!("WatchReason({}: {})", self.kind, self.text)
    }

    /// Refuses `copy` and `pickle`. See [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Where a followed device moved, as a watch took it (#2795): its own
/// reads named the device at `to`, and its old address no longer answered
/// as it. Frozen; refuses `copy` and `pickle`.
#[pyclass(name = "WatchMove", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWatchMove {
    from: String,
    to: String,
    timestamp: f64,
    count: u64,
}

impl PyWatchMove {
    /// `moved`, its addresses shown in `style`.
    fn new(moved: &Moved, style: AddressStyle) -> Self {
        Self {
            from: moved.from.styled(style),
            to: moved.to.styled(style),
            timestamp: crate::frames::posix_seconds(moved.when),
            count: moved.count,
        }
    }
}

#[pymethods]
impl PyWatchMove {
    /// Where the device answered before, as text `BACnetClient.device`
    /// reads back. `from_`, as `from` is a keyword.
    #[getter]
    fn from_(&self) -> &str {
        &self.from
    }

    /// Where it answers now, as text `BACnetClient.device` reads back.
    #[getter]
    fn to(&self) -> &str {
        &self.to
    }

    /// How many moves the watch has taken for the point, this one
    /// included: compare it to tell a new move.
    #[getter]
    fn count(&self) -> u64 {
        self.count
    }

    /// When the watch took the move, as POSIX seconds by the wall clock, as
    /// `time.time()`.
    #[getter]
    fn timestamp(&self) -> f64 {
        self.timestamp
    }

    fn __repr__(&self) -> String {
        format!("WatchMove({} -> {})", self.from, self.to)
    }

    /// Refuses `copy` and `pickle`. See [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
