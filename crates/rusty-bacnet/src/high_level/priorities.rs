//! What the priority calls take and return (#2561): a priority argument
//! ([`priority_arg`]), the client's [`PyWritePolicy`], an object's
//! commands as read ([`PyPriorityView`]) and a write the device
//! acknowledged ([`PyWritten`]).

use bacnet_ops::label::object_label;
use bacnet_ops::target::AddressStyle;
use bacnet_ops::write::Commandability;
use bacnet_ops::{Priority, PriorityView, WritePolicy, Written};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyInt, PyString, PyTuple};

use super::value::PyValue;
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

/// A `priority` argument: an int from 1 to 16. A bool, a float or anything
/// else that isn't an int raises TypeError; any other int, however large
/// or negative, ValueError.
pub(crate) fn priority_arg(value: &Bound<'_, PyAny>) -> PyResult<Priority> {
    if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(format!(
            "priority must be an int from 1 to 16, not {}",
            value.get_type().name()?
        )));
    }
    value
        .extract::<u8>()
        .ok()
        .and_then(|level| Priority::new(level).ok())
        .ok_or_else(|| PyValueError::new_err(format!("priority must be 1 to 16, not {value}")))
}

/// The priorities a priority view or a policy names, 1 first.
fn levels(priorities: impl Iterator<Item = Priority>) -> Vec<u8> {
    priorities.map(Priority::get).collect()
}

/// Which command priorities a `BACnetClient`'s handles may write and
/// relinquish at, given
/// with its `write_policy=` keyword. `WritePolicy()` refuses 1 and 2 (life
/// safety), 5 (critical equipment) and 6 (minimum on and off times);
/// `allow_priorities` gives a policy that allows some of them as well. The
/// check costs no traffic: a refused call raises BacnetWriteDeniedError
/// and sends nothing. `write_property` and the other typed calls don't
/// look at it.
#[pyclass(
    name = "WritePolicy",
    module = "rusty_bacnet",
    frozen,
    immutable_type,
    skip_from_py_object
)]
#[derive(Clone, Copy)]
pub(crate) struct PyWritePolicy(pub(crate) WritePolicy);

#[pymethods]
impl PyWritePolicy {
    /// The priorities the default refuses: (1, 2, 5, 6).
    #[classattr]
    #[pyo3(name = "GUARDED")]
    fn guarded(py: Python<'_>) -> PyResult<Py<PyTuple>> {
        Ok(PyTuple::new(py, levels(WritePolicy::GUARDED.into_iter()))?.unbind())
    }

    /// The default policy, refusing `GUARDED`.
    #[new]
    pub(crate) fn new() -> Self {
        Self(WritePolicy::default())
    }

    /// The same policy, allowing `priorities` (ints from 1 to 16) as well.
    /// One that isn't raises TypeError or ValueError, as `relinquish`'s
    /// `priority` does.
    pub(crate) fn allow_priorities(&self, priorities: &Bound<'_, PyAny>) -> PyResult<Self> {
        let allowed = priorities
            .try_iter()?
            .map(|item| priority_arg(&item?))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self(self.0.allow_priorities(allowed)))
    }

    /// Whether a relinquish at `priority` may go.
    pub(crate) fn allows(&self, priority: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.0.allows(priority_arg(priority)?))
    }

    /// The priorities refused, as a tuple, 1 first.
    #[getter]
    fn refused<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.refused_levels())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|other| other.get().0 == self.0)
    }

    pub(crate) fn __hash__(&self) -> u64 {
        self.refused_levels()
            .iter()
            .fold(0, |bits, level| bits | 1 << level)
    }

    pub(crate) fn __repr__(&self) -> String {
        format!("WritePolicy(refused={:?})", Tuple(&self.refused_levels()))
    }

    /// Copy and pickle as the calls that make an equal policy: `WritePolicy()`,
    /// then `allow_priorities` of the guarded priorities it allows, when it
    /// allows any. Every policy is one of those, since a policy only ever
    /// allows more than the default.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let policy = slf.get().0;
        let allowed = levels(
            WritePolicy::GUARDED
                .into_iter()
                .filter(|priority| policy.allows(*priority)),
        );
        if allowed.is_empty() {
            return Ok((slf.get_type().into_any(), PyTuple::empty(py)));
        }
        let allow = py
            .import("operator")?
            .getattr("methodcaller")?
            .call1(("allow_priorities", PyTuple::new(py, allowed)?))?;
        let default = Bound::new(py, Self::new())?;
        Ok((allow, PyTuple::new(py, [default])?))
    }
}

impl PyWritePolicy {
    fn refused_levels(&self) -> Vec<u8> {
        levels(Priority::all().filter(|p| !self.0.allows(*p)))
    }
}

/// A list of levels written as a Python tuple is: `(1, 2)`, `(1,)`, `()`.
struct Tuple<'a>(&'a [u8]);

impl std::fmt::Debug for Tuple<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            [one] => write!(f, "({one},)"),
            levels => {
                let shown: Vec<String> = levels.iter().map(u8::to_string).collect();
                write!(f, "({})", shown.join(", "))
            }
        }
    }
}

/// An object's commands as read: its Priority_Array's sixteen slots, the
/// slot in control, Relinquish_Default and Present_Value
/// (`Device.priority_array` and `Device.relinquish`).
/// Each value is a `Value`; values name no units. Refuses `copy` and
/// `pickle`.
#[pyclass(name = "PriorityView", module = "rusty_bacnet", frozen)]
pub(crate) struct PyPriorityView {
    object: ObjectIdentifier,
    /// Sixteen `Value`s or None, priority 1 first.
    slots: Py<PyTuple>,
    in_control: Option<Priority>,
    relinquish_default: Option<Py<PyValue>>,
    present_value: Py<PyValue>,
}

impl PyPriorityView {
    /// `view` as Python objects, built once.
    pub(crate) fn new(py: Python<'_>, view: &PriorityView) -> PyResult<Self> {
        let object_type = view.object.object_type();
        let slots = Priority::all()
            .map(|priority| {
                view.slot(priority)
                    .map(|value| {
                        let index = Some(u32::from(priority.get()));
                        let slot = PyValue::new(
                            object_type,
                            PropertyIdentifier::PRIORITY_ARRAY,
                            index,
                            value,
                        );
                        Py::new(py, slot).map(Py::into_any)
                    })
                    .transpose()
                    .map(|slot| slot.unwrap_or_else(|| py.None()))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let whole = |property, value| Py::new(py, PyValue::new(object_type, property, None, value));
        Ok(Self {
            object: view.object,
            slots: PyTuple::new(py, slots)?.unbind(),
            in_control: view.in_control,
            relinquish_default: view
                .relinquish_default
                .as_ref()
                .map(|value| whole(PropertyIdentifier::RELINQUISH_DEFAULT, value))
                .transpose()?,
            present_value: whole(PropertyIdentifier::PRESENT_VALUE, &view.present_value)?,
        })
    }
}

#[pymethods]
impl PyPriorityView {
    /// The object read.
    #[getter]
    fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// The sixteen slots as a tuple, priority 1 first (`slots[7]` is
    /// priority 8): each a `Value`, or None for an empty slot.
    #[getter]
    fn slots(&self, py: Python<'_>) -> Py<PyTuple> {
        self.slots.clone_ref(py)
    }

    /// The slot in control: the lowest-numbered priority holding a command.
    /// None when every slot is empty and Present_Value follows
    /// Relinquish_Default.
    #[getter]
    fn in_control(&self) -> Option<u8> {
        self.in_control.map(Priority::get)
    }

    /// Relinquish_Default; None when the device gave none.
    #[getter]
    fn relinquish_default(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.relinquish_default.as_ref().map(|v| v.clone_ref(py))
    }

    /// Present_Value.
    #[getter]
    fn present_value(&self, py: Python<'_>) -> Py<PyValue> {
        self.present_value.clone_ref(py)
    }

    /// The command at `priority` (an int from 1 to 16), None when its slot
    /// is empty.
    fn slot<'py>(
        &self,
        py: Python<'py>,
        priority: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let priority = priority_arg(priority)?;
        self.slots.bind(py).get_item(priority.index())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let quoted = |value: &Py<PyValue>| -> PyResult<String> {
            Ok(PyString::new(py, value.get().text()).repr()?.to_string())
        };
        let mut held = Vec::new();
        for (slot, priority) in self.slots.bind(py).iter().zip(Priority::all()) {
            if let Ok(value) = slot.cast::<PyValue>() {
                held.push(format!("{priority}: {}", quoted(&value.clone().unbind())?));
            }
        }
        let in_control = self
            .in_control
            .map_or_else(|| "None".to_string(), |p| p.to_string());
        let default = match &self.relinquish_default {
            Some(value) => quoted(value)?,
            None => "None".into(),
        };
        Ok(format!(
            "PriorityView({}, in_control={in_control}, present_value={}, \
             relinquish_default={default}, slots={{{}}})",
            object_label(self.object),
            quoted(&self.present_value)?,
            held.join(", ")
        ))
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A write the device acknowledged, as it was sent: where it went, the
/// property, the value and the priority. What `Device.write` returns, and
/// a BacnetReadBackError's and a BacnetNoPriorityArrayError's `written`.
/// Refuses `copy` and `pickle`.
#[pyclass(name = "Written", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWritten {
    address: String,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Py<PyValue>,
    priority: Option<Priority>,
    commandable: Commandability,
}

/// Whether a property takes command priorities, as Python names it.
pub(crate) fn commandable_label(commandable: Commandability) -> &'static str {
    match commandable {
        Commandability::Always => "always",
        Commandability::Never => "never",
        // Optional, and whatever a later release adds: only a read tells.
        _ => "optional",
    }
}

impl PyWritten {
    /// `written` as Python objects, its address as a link of `style`
    /// writes it: six octets on BACnet/SC are a VMAC, in hex, as the
    /// binding writes one elsewhere, not an IPv4 address and port.
    pub(crate) fn new(py: Python<'_>, written: &Written, style: AddressStyle) -> PyResult<Self> {
        let property = written.property;
        let value = PyValue::new(
            property.object.object_type(),
            property.property,
            property.index,
            &written.value,
        );
        Ok(Self {
            address: written.device.styled(style),
            object: property.object,
            property: property.property,
            index: property.index,
            value: Py::new(py, value)?,
            priority: written.priority,
            commandable: written.commandable,
        })
    }
}

#[pymethods]
impl PyWritten {
    /// The device's address the write went to, as `"ip:port"`, `"mstp:N"`,
    /// a B/IPv6 VMAC (`"bip6:0x0004D2"`) or an SC VMAC in hex
    /// (`"02:00:00:00:00:04"`).
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The object written.
    #[getter]
    fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// The property written.
    #[getter]
    fn property_id(&self) -> PyPropertyIdentifier {
        PyPropertyIdentifier {
            inner: self.property,
        }
    }

    /// The array index written, None for the whole property.
    #[getter]
    fn array_index(&self) -> Option<u32> {
        self.index
    }

    /// The value as sent, a `Value` (NULL for a relinquish).
    #[getter]
    fn value(&self, py: Python<'_>) -> Py<PyValue> {
        self.value.clone_ref(py)
    }

    /// The priority sent: the one asked for, or 16 for a property the
    /// standard makes commandable when none was. None when none was sent:
    /// a property that isn't commandable, or one that may be, written
    /// without one (the device then takes it at 16 if it is).
    #[getter]
    fn priority(&self) -> Option<u8> {
        self.priority.map(Priority::get)
    }

    /// Whether the property takes priorities, as its object type and
    /// identifier say with nothing read: `"always"` (an output's
    /// Present_Value), `"optional"` (a value object's, commandable only
    /// when the object has a Priority_Array) or `"never"`.
    #[getter]
    fn commandable(&self) -> &'static str {
        commandable_label(self.commandable)
    }

    fn __repr__(&self) -> String {
        let priority = self
            .priority
            .map(|p| format!(" at priority {p}"))
            .unwrap_or_default();
        let index = self.index.map(|i| format!("[{i}]")).unwrap_or_default();
        format!(
            "Written({} {}{index} = {}{priority}, to {})",
            object_label(self.object),
            bacnet_ops::label::kebab(self.property),
            self.value.get().text(),
            self.address
        )
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
