//! The experimental Standard 223 export (#2436), as `bacnet-ops` reads and
//! writes it: a device's objects read into plain data over the binding's
//! own client, and Turtle written from that data on request.

use std::sync::atomic::AtomicBool;

use bacnet_client::client::BACnetClient;
use bacnet_ops::engine::{Halt, Reader};
use bacnet_ops::export::s223::{
    Access, DEFAULT_BASE_IRI, Entry, Inventory, InventoryOptions, Point, PointKind, TurtleOptions,
    WritePriority, read_inventory, to_turtle,
};
use bacnet_ops::limits::RequestLimits;
use bacnet_types::enums::EngineeringUnits;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::app_transport::AppTransport;
use crate::errors::to_py_err;
use crate::types::{IntEnumValue, PyObjectIdentifier, PyObjectType};

/// The options `types=` gives: every exported type when `None`, refused
/// with the core's message when one isn't exported or the list is empty.
pub(crate) fn options(types: Option<Vec<PyObjectType>>) -> PyResult<InventoryOptions> {
    match types {
        None => Ok(InventoryOptions::default()),
        Some(types) => InventoryOptions::new(types.iter().map(PyObjectType::to_rust).collect())
            .map_err(PyValueError::new_err),
    }
}

/// `device`'s inventory over `client`, one request at a time with no rate
/// cap beyond the client's own `min_request_interval_ms`, as the CLI reads
/// it. A device that doesn't answer the name read, or a failed read of
/// the whole Object_List, raises the client's usual error; a failure after
/// that ends the reading with `stopped` set.
pub(crate) async fn read(
    client: &BACnetClient<AppTransport>,
    device: u32,
    options: &InventoryOptions,
) -> PyResult<PyInventory223> {
    let limits = RequestLimits::new(0);
    // Nothing sets the flag: a Python caller cancels by cancelling the task,
    // which drops this future.
    let never = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device,
        rate: &limits,
        cancel: &never,
        rpm: true,
    };
    match read_inventory(&mut reader, options).await {
        // The objects' Python copies are built once, so indexing
        // `objects` in a loop doesn't copy the list each time.
        Ok(inventory) => crate::py_async::attach(|py| {
            let objects = inventory
                .entries
                .iter()
                .map(|entry| Py::new(py, PyObject223(entry.clone())))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyInventory223 {
                objects: PyTuple::new(py, objects)?.unbind(),
                inventory,
            })
        }),
        Err(Halt::Device(error)) => Err(to_py_err(error)),
        Err(Halt::Cancelled) => unreachable!("nothing sets the export's stop flag"),
    }
}

/// A device's objects as the experimental Standard 223 export read them
/// (`BACnetClient.export_223_experimental`). Device text is kept exactly
/// as the device sent it. Refuses `copy` and `pickle`.
#[pyclass(name = "Inventory223Experimental", module = "rusty_bacnet", frozen)]
pub(crate) struct PyInventory223 {
    inventory: Inventory,
    /// `inventory.entries` as Python objects, built once.
    objects: Py<PyTuple>,
}

#[pymethods]
impl PyInventory223 {
    /// The device instance.
    #[getter]
    fn device_instance(&self) -> u32 {
        self.inventory.device.instance
    }

    /// The Device object's Object_Name, exactly as the device sent it;
    /// None when it couldn't be read.
    #[getter]
    fn device_name(&self) -> Option<&str> {
        self.inventory.device.name.as_deref()
    }

    /// The objects read, in Object_List order, as a tuple.
    #[getter]
    fn objects(&self, py: Python<'_>) -> Py<PyTuple> {
        self.objects.clone_ref(py)
    }

    /// Why the reading stopped early, None when it didn't. A request to the
    /// device timed out or its link failed, so the objects after that point
    /// are missing; or a read of the Object_List element by element failed
    /// partway (an error or no answer), and then `objects` is empty.
    #[getter]
    fn stopped(&self) -> Option<&str> {
        self.inventory.stopped.as_deref()
    }

    /// The inventory as Turtle. `base_iri` defaults to
    /// `http://example.org/bacnet/`; each device's nodes go under
    /// `<base_iri>device-<instance>/`. `write_priority` (1 to 16) goes in
    /// the reference of every commandable point. A bad base IRI or a
    /// priority outside 1 to 16 raises ValueError saying which rule failed;
    /// a priority outside 0 to 255 raises OverflowError.
    #[pyo3(signature = (base_iri=None, write_priority=None))]
    fn to_turtle(&self, base_iri: Option<&str>, write_priority: Option<u8>) -> PyResult<String> {
        let priority = write_priority
            .map(WritePriority::new)
            .transpose()
            .map_err(PyValueError::new_err)?;
        let options = TurtleOptions::new(base_iri.unwrap_or(DEFAULT_BASE_IRI), priority)
            .map_err(PyValueError::new_err)?;
        Ok(to_turtle(&self.inventory, &options))
    }

    fn __repr__(&self) -> String {
        let stopped = if self.inventory.stopped.is_some() {
            ", stopped"
        } else {
            ""
        };
        format!(
            "Inventory223Experimental(device={}, objects={}{stopped})",
            self.inventory.device.instance,
            self.inventory.entries.len()
        )
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// One object of an `Inventory223Experimental`: a point, or an object
/// whose read failed, which has only its identifier and `error`.
#[pyclass(name = "Object223Experimental", module = "rusty_bacnet", frozen)]
pub(crate) struct PyObject223(Entry);

impl PyObject223 {
    fn point(&self) -> Option<&Point> {
        self.0.point()
    }
}

#[pymethods]
impl PyObject223 {
    /// The object.
    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.0.object())
    }

    /// Its Object_Name; None when its read failed.
    #[getter]
    fn name(&self) -> Option<&str> {
        self.point().map(|point| point.name.as_str())
    }

    /// Its Description; None when it has none or couldn't be read.
    #[getter]
    fn description(&self) -> Option<&str> {
        self.point()?.description.as_deref()
    }

    /// The Units of an analog object or an integer value; None for other
    /// objects or when its read failed.
    #[getter]
    fn units(&self) -> Option<IntEnumValue<EngineeringUnits>> {
        match &self.point()?.kind {
            PointKind::Analog(analog) => Some(IntEnumValue(analog.units)),
            PointKind::Integer(integer) => Some(IntEnumValue(integer.units)),
            _ => None,
        }
    }

    /// A multi-state object's Number_Of_States; None for other objects or
    /// when its read failed.
    #[getter]
    fn number_of_states(&self) -> Option<u32> {
        match &self.point()?.kind {
            PointKind::MultiState(states) => Some(states.number_of_states),
            _ => None,
        }
    }

    /// A multi-state object's State_Text, state 1 first; empty for other
    /// objects, when it has none or when its read failed. An entry that
    /// wasn't a string is "".
    #[getter]
    fn state_texts(&self) -> Vec<String> {
        match self.point().map(|point| &point.kind) {
            Some(PointKind::MultiState(states)) => states.state_texts.clone(),
            _ => Vec::new(),
        }
    }

    /// Whether it has a Priority_Array: always for an output, and for a
    /// value when the device gave one. The Turtle makes such a point
    /// actuatable.
    #[getter]
    fn commandable(&self) -> bool {
        match self.point().map(|point| &point.kind) {
            Some(PointKind::Analog(analog)) => matches!(analog.access, Access::Commandable { .. }),
            Some(PointKind::Binary(binary)) => matches!(binary.access, Access::Commandable { .. }),
            Some(PointKind::MultiState(states)) => {
                matches!(states.access, Access::Commandable { .. })
            }
            Some(PointKind::Integer(integer)) => {
                matches!(integer.access, Access::Commandable { .. })
            }
            _ => false,
        }
    }

    /// Why its read failed, None when it didn't: the device's error, which
    /// names the property when the object lacked its Object_Name, the Units
    /// of an analog object or an integer value, or a multi-state object's
    /// Number_Of_States. The Turtle skips such an object.
    #[getter]
    fn error(&self) -> Option<&str> {
        self.0.error()
    }

    fn __repr__(&self) -> String {
        format!(
            "Object223Experimental({}, name={:?})",
            self.0.object(),
            self.name().unwrap_or("")
        )
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Add the two classes to the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyInventory223>()?;
    m.add_class::<PyObject223>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use bacnet_ops::export::s223::{Analog, Integer};
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;

    /// An object that failed keeps its identifier and its error, which
    /// names the property; every other getter is empty. A point fills
    /// those its family has.
    #[test]
    fn a_failed_object_has_only_its_error() {
        let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 6).unwrap();
        let error = "units: PROPERTY:UNKNOWN_PROPERTY";
        let failed = PyObject223(Entry::Failed {
            object,
            property: Some(bacnet_types::enums::PropertyIdentifier::UNITS),
            error: error.into(),
        });
        assert_eq!(failed.error(), Some(error));
        assert_eq!((failed.name(), failed.description()), (None, None));
        assert!(failed.units().is_none() && failed.number_of_states().is_none());
        assert!(failed.state_texts().is_empty() && !failed.commandable());
        assert!(
            failed.__repr__().contains("name=\"\""),
            "{}",
            failed.__repr__()
        );

        let damper = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
        let commanded = Access::Commandable {
            relinquish_default: None,
        };
        let analog = Analog::new(EngineeringUnits::PERCENT, commanded);
        let point = PyObject223(Entry::Point(Point::new(damper, "Damper", analog)));
        assert_eq!((point.name(), point.error()), (Some("Damper"), None));
        assert!(
            point
                .units()
                .is_some_and(|units| units.0 == EngineeringUnits::PERCENT)
        );
        assert!(point.commandable() && point.number_of_states().is_none());

        // An integer value has Units too, and is commandable by its access.
        let meter = ObjectIdentifier::new(ObjectType::INTEGER_VALUE, 2).unwrap();
        let integer = Integer::new(EngineeringUnits::KILOWATT_HOURS, Access::Writable);
        let point = PyObject223(Entry::Point(Point::new(meter, "Meter", integer)));
        assert!(
            point
                .units()
                .is_some_and(|units| units.0 == EngineeringUnits::KILOWATT_HOURS)
        );
        assert!(!point.commandable() && point.number_of_states().is_none());
        let commanded = Integer::new(
            EngineeringUnits::NO_UNITS,
            Access::Commandable {
                relinquish_default: None,
            },
        );
        let point = PyObject223(Entry::Point(Point::new(meter, "Meter", commanded)));
        assert!(point.commandable());
    }
}
