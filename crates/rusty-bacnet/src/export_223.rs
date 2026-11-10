//! The experimental Standard 223 export (#2436), as `bacnet-ops` reads and
//! writes it: a device's objects read into plain data over the binding's
//! own client, and Turtle written from that data on request.

use std::sync::atomic::AtomicBool;

use bacnet_client::client::BACnetClient;
use bacnet_ops::engine::{Halt, Reader};
use bacnet_ops::export::s223::{
    DEFAULT_BASE_IRI, Inventory, InventoryObject, InventoryOptions, TurtleOptions, WritePriority,
    read_inventory, to_turtle,
};
use bacnet_ops::limits::RequestLimits;
use bacnet_types::enums::EngineeringUnits;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

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
/// it. A failure before the Object_List is in raises the client's usual
/// error; one after it ends the reading with `stopped` set.
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
        Ok(inventory) => Ok(PyInventory223(inventory)),
        Err(Halt::Device(error)) => Err(to_py_err(error)),
        Err(Halt::Cancelled) => unreachable!("nothing sets the export's stop flag"),
    }
}

/// A device's objects as the experimental Standard 223 export read them
/// (`BACnetClient.export_223_experimental`). Device text is kept exactly
/// as the device sent it. Refuses `copy` and `pickle`.
#[pyclass(name = "Inventory223Experimental", module = "rusty_bacnet", frozen)]
pub(crate) struct PyInventory223(Inventory);

#[pymethods]
impl PyInventory223 {
    /// The device instance.
    #[getter]
    fn device_instance(&self) -> u32 {
        self.0.device
    }

    /// The Device object's Object_Name, exactly as the device sent it;
    /// None when it couldn't be read.
    #[getter]
    fn device_name(&self) -> Option<&str> {
        self.0.device_name.as_deref()
    }

    /// The objects read, in Object_List order.
    #[getter]
    fn objects(&self) -> Vec<PyObject223> {
        self.0.objects.iter().cloned().map(PyObject223).collect()
    }

    /// Why the reading stopped early, None when it didn't: the device
    /// stopped answering. The objects after that point are missing.
    #[getter]
    fn stopped(&self) -> Option<&str> {
        self.0.stopped.as_deref()
    }

    /// The inventory as Turtle. `base_iri` defaults to
    /// `http://example.org/bacnet/`; each device's nodes go under
    /// `<base_iri>device-<instance>/`. `write_priority` (1 to 16) goes in
    /// the reference of every commandable point. A bad base IRI or
    /// priority raises ValueError saying which rule failed.
    #[pyo3(signature = (base_iri=None, write_priority=None))]
    fn to_turtle(&self, base_iri: Option<&str>, write_priority: Option<u8>) -> PyResult<String> {
        let priority = write_priority
            .map(WritePriority::new)
            .transpose()
            .map_err(PyValueError::new_err)?;
        let options = TurtleOptions::new(base_iri.unwrap_or(DEFAULT_BASE_IRI), priority)
            .map_err(PyValueError::new_err)?;
        Ok(to_turtle(&self.0, &options))
    }

    fn __repr__(&self) -> String {
        let stopped = if self.0.stopped.is_some() {
            ", stopped"
        } else {
            ""
        };
        format!(
            "Inventory223Experimental(device={}, objects={}{stopped})",
            self.0.device,
            self.0.objects.len()
        )
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// One object of an `Inventory223Experimental`.
#[pyclass(name = "Object223Experimental", module = "rusty_bacnet", frozen)]
pub(crate) struct PyObject223(InventoryObject);

#[pymethods]
impl PyObject223 {
    /// The object.
    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.0.object)
    }

    /// Its Object_Name; None when it couldn't be read.
    #[getter]
    fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }

    /// Its Description; None when it has none or couldn't be read.
    #[getter]
    fn description(&self) -> Option<&str> {
        self.0.description.as_deref()
    }

    /// An analog object's Units; None for other objects or when not read.
    #[getter]
    fn units(&self) -> Option<IntEnumValue<EngineeringUnits>> {
        self.0.units.map(IntEnumValue)
    }

    /// A multi-state object's Number_Of_States; None for other objects or
    /// when not read.
    #[getter]
    fn number_of_states(&self) -> Option<u32> {
        self.0.states
    }

    /// A multi-state object's State_Text, state 1 first; empty for other
    /// objects or when not read. An entry that wasn't a string is "".
    #[getter]
    fn state_texts(&self) -> Vec<String> {
        self.0.state_texts.clone()
    }

    /// Whether it has a Priority_Array: always for an output, and for a
    /// value when the device gave one. The Turtle makes such a point
    /// actuatable.
    #[getter]
    fn commandable(&self) -> bool {
        self.0.priority_array
    }

    /// Why its read failed, None when it didn't; the Turtle skips such an
    /// object.
    #[getter]
    fn error(&self) -> Option<&str> {
        self.0.error.as_deref()
    }

    fn __repr__(&self) -> String {
        format!(
            "Object223Experimental({}, name={:?})",
            self.0.object,
            self.0.name.as_deref().unwrap_or("")
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
