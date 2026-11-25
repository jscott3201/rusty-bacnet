//! A device's objects as point records (#2561): [`PyPoint`], one object's
//! record as `bacnet_ops::inventory` reads it, and [`PyInventory`], what
//! `Device.inventory` read. `Device.points` hands the same records out one
//! batch at a time ([`super::walk`]).
//!
//! A record is plain data, decoded: units as an `EngineeringUnits` and as
//! the tool's text, state texts as a tuple, each value read with `values`
//! or `settings` as a `Value`. A property the device couldn't give is a
//! problem on the record, the exception a typed read of it would raise,
//! never raised. Text the device supplied (names, descriptions, state
//! texts, Device_Type, Profile_Name) is kept exactly as it sent it.

use std::collections::{HashMap, HashSet};

use bacnet_ops::inventory::{Inventory, InventoryOptions, PointRecord};
use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::parse::parse_object_type;
use bacnet_ops::target::AddressStyle;
use bacnet_ops::{CovSupport, Value};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple};

use super::errors::ops_err;
use super::readings::{PyDeviceSummary, row_error};
use super::specs::{object_arg, property_part, refuse_text};
use super::value::PyValue;
use crate::types::{IntEnumValue, PyObjectIdentifier, PyObjectType, PyPropertyIdentifier};

#[cfg(test)]
#[path = "points_tests.rs"]
mod tests;

/// What is known of a device's COV support, as Python names it.
pub(super) fn cov_label(support: CovSupport) -> &'static str {
    match support {
        CovSupport::NotOffered => "not-offered",
        CovSupport::Offered => "offered",
        CovSupport::Accepted => "accepted",
        CovSupport::Refused => "refused",
        _ => "unknown",
    }
}

/// The options the `types`, `values`, `settings` and `details` keywords
/// give, checked when the call is made: `types` an iterable of
/// `ObjectType`s or their text (`"ai"`, `"analog-input"`), not one str,
/// and not empty (None reads every type).
pub(super) fn options(
    types: Option<&Bound<'_, PyAny>>,
    values: bool,
    settings: bool,
    details: bool,
) -> PyResult<InventoryOptions> {
    let options = InventoryOptions::new()
        .with_values(values)
        .with_settings(settings)
        .with_details(details);
    let Some(types) = types.filter(|types| !types.is_none()) else {
        return Ok(options);
    };
    refuse_text(types, "types", "['ai', ObjectType.ANALOG_OUTPUT]")?;
    let types = types
        .try_iter()?
        .map(|item| object_type(&item?))
        .collect::<PyResult<Vec<_>>>()?;
    if types.is_empty() {
        return Err(PyValueError::new_err(
            "types names no object type: leave it out to read every type",
        ));
    }
    Ok(options.with_types(types))
}

/// One `types` item: an `ObjectType`, or its text.
fn object_type(item: &Bound<'_, PyAny>) -> PyResult<ObjectType> {
    if let Ok(object_type) = item.cast::<PyObjectType>() {
        return Ok(object_type.get().to_rust());
    }
    if let Ok(text) = item.cast::<PyString>() {
        return parse_object_type(text.to_str()?).map_err(PyValueError::new_err);
    }
    Err(PyTypeError::new_err(format!(
        "an object type is an ObjectType or text such as 'ai', not {}",
        item.get_type().name()?
    )))
}

/// An `objects` argument: an iterable of `ObjectIdentifier`s or their text,
/// each kept once, in order.
pub(super) fn objects_arg(objects: &Bound<'_, PyAny>) -> PyResult<Vec<ObjectIdentifier>> {
    refuse_text(objects, "objects", "['ai:1', 'ao:2']")?;
    let named = objects
        .try_iter()?
        .map(|item| object_arg(&item?))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(once_each(named, |_| true))
}

/// `objects` in order, each once, those `keep` keeps.
pub(super) fn once_each(
    objects: Vec<ObjectIdentifier>,
    keep: impl Fn(ObjectIdentifier) -> bool,
) -> Vec<ObjectIdentifier> {
    let mut seen = HashSet::new();
    objects
        .into_iter()
        .filter(|object| keep(*object) && seen.insert(*object))
        .collect()
}

/// One object of a device as an inventory reads it: where it is, what it
/// is called, its units or state texts, whether it takes commands, and
/// what else the options read. Frozen; refuses `copy` and `pickle`, as the
/// other read results do, and its `Value`s copy and pickle.
#[pyclass(name = "Point", module = "rusty_bacnet", frozen)]
pub(crate) struct PyPoint {
    address: String,
    device_instance: u32,
    object: ObjectIdentifier,
    name: Option<String>,
    description: Option<String>,
    device_type: Option<String>,
    profile_name: Option<String>,
    units: Option<EngineeringUnits>,
    active_text: Option<String>,
    inactive_text: Option<String>,
    state_texts: Vec<String>,
    number_of_states: Option<u32>,
    commandable: bool,
    /// The values read with settings or values, in [`VALUES`] order.
    values: [Option<Py<PyValue>>; VALUES.len()],
    /// `(PropertyIdentifier, error)` pairs, in the order asked.
    problems: Py<PyTuple>,
}

/// The properties a record's [`Value`]s are values of, in the order
/// [`PyPoint::values`] keeps them.
const VALUES: [PropertyIdentifier; 10] = [
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::COV_INCREMENT,
    PropertyIdentifier::MIN_PRES_VALUE,
    PropertyIdentifier::MAX_PRES_VALUE,
    PropertyIdentifier::POLARITY,
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::RELIABILITY,
];

impl PyPoint {
    /// `record` as Python reads it, its device's address in `style`.
    pub(super) fn new(py: Python<'_>, record: &PointRecord, style: AddressStyle) -> PyResult<Self> {
        let object_type = record.object.object_type();
        let read = [
            &record.relinquish_default,
            &record.cov_increment,
            &record.min_present_value,
            &record.max_present_value,
            &record.polarity,
            &record.present_value,
            &record.command_priority,
            &record.status_flags,
            &record.event_state,
            &record.reliability,
        ];
        let mut values: [Option<Py<PyValue>>; VALUES.len()] = Default::default();
        for ((slot, value), property) in values.iter_mut().zip(read).zip(VALUES) {
            *slot = value
                .as_ref()
                .map(|value: &Value| Py::new(py, PyValue::new(object_type, property, None, value)))
                .transpose()?;
        }
        let problems = record
            .problems
            .iter()
            .map(|problem| {
                let property = PyPropertyIdentifier {
                    inner: problem.property,
                };
                (property, row_error(py, &problem.error)).into_pyobject(py)
            })
            .collect::<PyResult<Vec<_>>>()?;
        let texts = &record.state_texts;
        Ok(Self {
            address: record.device.styled(style),
            device_instance: record.device_instance,
            object: record.object,
            name: record.name.clone(),
            description: record.description.clone(),
            device_type: record.device_type.clone(),
            profile_name: record.profile_name.clone(),
            units: record.units,
            active_text: texts.active.clone(),
            inactive_text: texts.inactive.clone(),
            state_texts: texts.states.clone(),
            number_of_states: record.number_of_states,
            commandable: record.commandable,
            values,
            problems: PyTuple::new(py, problems)?.unbind(),
        })
    }

    /// The value read of `property`, None when it wasn't.
    fn value(&self, py: Python<'_>, property: PropertyIdentifier) -> Option<Py<PyValue>> {
        let at = VALUES.iter().position(|held| *held == property)?;
        self.values[at].as_ref().map(|value| value.clone_ref(py))
    }
}

#[pymethods]
impl PyPoint {
    /// Where the device answers, as text `BACnetClient.device` reads back.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The device's instance.
    #[getter]
    fn device_instance(&self) -> u32 {
        self.device_instance
    }

    /// The object.
    #[getter]
    fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// Its Object_Name. Text the device supplied, exactly as it sent it.
    #[getter]
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Its Description. Text the device supplied, exactly as it sent it.
    #[getter]
    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Its Device_Type, read with `details`. Text the device supplied.
    #[getter]
    fn device_type(&self) -> Option<&str> {
        self.device_type.as_deref()
    }

    /// Its Profile_Name, read with `details`. Text the device supplied.
    #[getter]
    fn profile_name(&self) -> Option<&str> {
        self.profile_name.as_deref()
    }

    /// Its Units, for the types that have them, as an `EngineeringUnits`.
    #[getter]
    fn units(&self) -> Option<IntEnumValue<EngineeringUnits>> {
        self.units.map(IntEnumValue)
    }

    /// Its Units as the `bacnet` tool writes them (`"degrees-celsius"`);
    /// a unit without a name as its number.
    #[getter]
    fn units_text(&self) -> Option<String> {
        self.units.map(kebab)
    }

    /// A binary object's Active_Text. Text the device supplied.
    #[getter]
    fn active_text(&self) -> Option<&str> {
        self.active_text.as_deref()
    }

    /// A binary object's Inactive_Text. Text the device supplied.
    #[getter]
    fn inactive_text(&self) -> Option<&str> {
        self.inactive_text.as_deref()
    }

    /// A multi-state object's State_Text, state 1 first; empty for other
    /// types. Text the device supplied; an entry that wasn't a string is
    /// "" and a problem.
    #[getter]
    fn state_texts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.state_texts)
    }

    /// A multi-state object's Number_Of_States.
    #[getter]
    fn number_of_states(&self) -> Option<u32> {
        self.number_of_states
    }

    /// Whether its Present_Value takes command priorities: always for an
    /// output, and for a value object when it has a Priority_Array.
    #[getter]
    fn commandable(&self) -> bool {
        self.commandable
    }

    /// Its Relinquish_Default, read with `settings`.
    #[getter]
    fn relinquish_default(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::RELINQUISH_DEFAULT)
    }

    /// Its COV_Increment, read with `settings`.
    #[getter]
    fn cov_increment(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::COV_INCREMENT)
    }

    /// Its Min_Pres_Value, read with `settings`.
    #[getter]
    fn min_present_value(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::MIN_PRES_VALUE)
    }

    /// Its Max_Pres_Value, read with `settings`.
    #[getter]
    fn max_present_value(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::MAX_PRES_VALUE)
    }

    /// A Binary Input's or Output's Polarity, read with `settings`.
    #[getter]
    fn polarity(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::POLARITY)
    }

    /// Its Present_Value, read with `values`; its text names the units or
    /// the state.
    #[getter]
    fn present_value(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::PRESENT_VALUE)
    }

    /// The priority in control of a commandable object's Present_Value,
    /// read with `values`: 1 to 16, or NULL when Relinquish_Default
    /// applies.
    #[getter]
    fn command_priority(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::CURRENT_COMMAND_PRIORITY)
    }

    /// Its Status_Flags, read with `values`.
    #[getter]
    fn status_flags(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::STATUS_FLAGS)
    }

    /// Its Event_State, read with `values`.
    #[getter]
    fn event_state(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::EVENT_STATE)
    }

    /// Its Reliability, read with `values`.
    #[getter]
    fn reliability(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value(py, PropertyIdentifier::RELIABILITY)
    }

    /// The properties asked for that gave no usable value, in the order
    /// asked, as `(PropertyIdentifier, error)` pairs: the error is the
    /// exception a typed read would raise for it, or a `BacnetError`
    /// saying what was wrong with the value; never raised.
    #[getter]
    fn problems(&self, py: Python<'_>) -> Py<PyTuple> {
        self.problems.clone_ref(py)
    }

    /// The problem with `property` (a `PropertyIdentifier` or its text),
    /// None when it had none. A problem is a whole property's: text that
    /// names an element raises ValueError.
    fn problem(&self, py: Python<'_>, property: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyAny>>> {
        let (wanted, element) = property_part(property)?;
        if element.is_some() {
            return Err(PyValueError::new_err(
                "a problem is a whole property's: name the property without an element",
            ));
        }
        for pair in self.problems.bind(py).iter() {
            let (held, error): (PyRef<'_, PyPropertyIdentifier>, Py<PyAny>) = pair.extract()?;
            if held.to_rust() == wanted {
                return Ok(Some(error));
            }
        }
        Ok(None)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let name = match &self.name {
            Some(name) => {
                // A long name is cut short, as a frame field's label is.
                let shown = match name.char_indices().nth(59) {
                    Some((cut, _)) => format!("{}…", &name[..cut]),
                    None => name.clone(),
                };
                PyString::new(py, &shown).repr()?.to_string()
            }
            None => "None".to_owned(),
        };
        Ok(format!("Point({}, name={name})", object_label(self.object)))
    }

    /// Refuses `copy` and `pickle`, as the other read results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// What `Device.inventory` read: the device's summary and a `Point` for
/// each object of the types asked for, in Object_List order, and, when
/// the device went silent part way, why and which objects weren't read.
/// Iterating it gives the points. Frozen; compares by identity and
/// refuses `copy` and `pickle`, as a `Point` does.
#[pyclass(name = "Inventory", module = "rusty_bacnet", frozen)]
pub(crate) struct PyInventory {
    address: String,
    instance: u32,
    summary: Py<PyDeviceSummary>,
    cov: CovSupport,
    points: Py<PyTuple>,
    /// Each point's place in `points`, by its object.
    places: HashMap<ObjectIdentifier, usize>,
    unread: Vec<ObjectIdentifier>,
    /// The exception a single read would have raised, unraised.
    stopped: Option<Py<PyAny>>,
}

impl PyInventory {
    /// `inventory` as Python reads it, addresses in `style`.
    fn new(py: Python<'_>, inventory: Inventory, style: AddressStyle) -> PyResult<Self> {
        let points = inventory
            .points
            .iter()
            .map(|record| Py::new(py, PyPoint::new(py, record, style)?))
            .collect::<PyResult<Vec<_>>>()?;
        let mut places = HashMap::with_capacity(inventory.points.len());
        for (at, record) in inventory.points.iter().enumerate() {
            places.entry(record.object).or_insert(at);
        }
        Ok(Self {
            address: inventory.address.styled(style),
            instance: inventory.device.device,
            summary: Py::new(py, PyDeviceSummary::new(py, &inventory.device)?)?,
            cov: inventory.cov,
            points: PyTuple::new(py, points)?.unbind(),
            places,
            unread: inventory.unread,
            stopped: inventory
                .stopped
                .map(|error| ops_err(error, style).into_value(py).into_any()),
        })
    }
}

/// An inventory as read, its addresses shown in the style: made an
/// `Inventory` when it reaches Python, so the points are built on the
/// bridge's blocking thread after the call has let the client go, not on
/// a runtime worker with the lease held.
pub(super) struct ReadInventory(pub(super) Box<Inventory>, pub(super) AddressStyle);

impl<'py> IntoPyObject<'py> for ReadInventory {
    type Target = PyInventory;
    type Output = Bound<'py, PyInventory>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyInventory>> {
        Bound::new(py, PyInventory::new(py, *self.0, self.1)?)
    }
}

#[pymethods]
impl PyInventory {
    /// Where the device answers, as text `BACnetClient.device` reads back.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The device's summary, as `Device.summary()` reads it. Its `name` is
    /// cleaned for display, where the Device object's `Point.name` is the
    /// device's text exactly as sent.
    #[getter]
    fn summary(&self, py: Python<'_>) -> Py<PyDeviceSummary> {
        self.summary.clone_ref(py)
    }

    /// What is known of the device's COV support, as
    /// `Device.cov_support()` says it: from the summary's services, or
    /// from what this client's subscriptions met.
    #[getter]
    fn cov_support(&self) -> &'static str {
        cov_label(self.cov)
    }

    /// The points read, as a tuple, in Object_List order.
    #[getter]
    fn points(&self, py: Python<'_>) -> Py<PyTuple> {
        self.points.clone_ref(py)
    }

    /// The objects of the types asked for that weren't read because the
    /// device stopped answering, as `ObjectIdentifier`s in Object_List
    /// order; empty when it didn't.
    #[getter]
    fn unread<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.unread
                .iter()
                .map(|o| PyObjectIdentifier::from_rust(*o)),
        )
    }

    /// Set when the device went silent, or the way to it failed, while
    /// its objects were being read: the BacnetTimeoutError or other error a
    /// single read would have raised, unraised. `points` holds what was
    /// read before it, and `unread` the rest.
    #[getter]
    fn stopped(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.stopped.as_ref().map(|error| error.clone_ref(py))
    }

    /// The point of `object` (an `ObjectIdentifier` or its text), None
    /// when it wasn't read.
    fn point(&self, py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyPoint>>> {
        let wanted = object_arg(object)?;
        let Some(&at) = self.places.get(&wanted) else {
            return Ok(None);
        };
        let point = self.points.bind(py).get_item(at)?;
        Ok(Some(point.cast_into::<PyPoint>()?.unbind()))
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.points.bind(py).len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.points
            .bind(py)
            .as_any()
            .try_iter()
            .map(Bound::into_any)
    }

    fn __getitem__<'py>(
        &self,
        py: Python<'py>,
        index: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.points.bind(py).as_any().get_item(index)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let stopped = if self.stopped.is_some() {
            ", stopped"
        } else {
            ""
        };
        format!(
            "Inventory(device={}, points={}{stopped})",
            self.instance,
            self.points.bind(py).len()
        )
    }

    /// Refuses `copy` and `pickle`, as the other read results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
