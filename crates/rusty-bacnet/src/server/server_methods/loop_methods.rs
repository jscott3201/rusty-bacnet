//! Loop registration and the application route for its measurement.
use super::super::*;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;

#[pymethods]
impl BACnetServer {
    /// Add a Loop (PID) object to the server (before starting).
    ///
    /// The keyword arguments set the rows that are read-only over the network:
    /// Controlled_Variable_Units and the three gain units (NO_UNITS when
    /// omitted) and Priority_For_Writing (16 when omitted). Units above 65535
    /// or a priority outside 1..=16 raise VALUE_OUT_OF_RANGE.
    #[pyo3(signature = (
        instance,
        name,
        output_units=62,
        *,
        controlled_variable_units=None,
        proportional_constant_units=None,
        integral_constant_units=None,
        derivative_constant_units=None,
        priority_for_writing=None
    ))]
    fn add_loop(
        &self,
        instance: u32,
        name: &str,
        output_units: u32,
        controlled_variable_units: Option<u32>,
        proportional_constant_units: Option<u32>,
        integral_constant_units: Option<u32>,
        derivative_constant_units: Option<u32>,
        priority_for_writing: Option<u8>,
    ) -> PyResult<()> {
        let lp = loop_object(
            instance,
            name,
            output_units,
            LoopSettings {
                controlled_variable_units,
                proportional_constant_units,
                integral_constant_units,
                derivative_constant_units,
                priority_for_writing,
            },
        )
        .map_err(to_py_err)?;
        self.push_pending(Box::new(lp))
    }

    /// Feed a Loop's measured Controlled_Variable_Value while the application
    /// runs the loop's algorithm.
    ///
    /// The value must be a finite REAL. Any object other than a Loop raises
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as in `set_present_value_local`.
    /// Out_Of_Service doesn't block it. A SubscribeCOVProperty on the property
    /// is notified; a SubscribeCOV on the Loop sees the value in its next report.
    #[pyo3(signature = (object_id, value))]
    fn set_controlled_variable_value_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        value: PyPropertyValue,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let prop_value = value.inner;

        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_controlled_variable_value_local(&oid, prop_value)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}

/// The optional `add_loop` keyword arguments; `None` keeps the Loop's default.
#[derive(Default)]
struct LoopSettings {
    controlled_variable_units: Option<u32>,
    proportional_constant_units: Option<u32>,
    integral_constant_units: Option<u32>,
    derivative_constant_units: Option<u32>,
    priority_for_writing: Option<u8>,
}

/// Build the Loop through the Rust setters, so Python gets their checks.
fn loop_object(
    instance: u32,
    name: &str,
    output_units: u32,
    settings: LoopSettings,
) -> Result<LoopObject, Error> {
    type UnitsSetter = fn(&mut LoopObject, EngineeringUnits) -> Result<(), Error>;
    let mut lp = LoopObject::new(instance, name, EngineeringUnits::from_raw(output_units))?;
    let units: [(Option<u32>, UnitsSetter); 4] = [
        (
            settings.controlled_variable_units,
            LoopObject::set_controlled_variable_units,
        ),
        (
            settings.proportional_constant_units,
            LoopObject::set_proportional_constant_units,
        ),
        (
            settings.integral_constant_units,
            LoopObject::set_integral_constant_units,
        ),
        (
            settings.derivative_constant_units,
            LoopObject::set_derivative_constant_units,
        ),
    ];
    for (value, set) in units {
        if let Some(raw) = value {
            set(&mut lp, EngineeringUnits::from_raw(raw))?;
        }
    }
    if let Some(priority) = settings.priority_for_writing {
        lp.set_priority_for_writing(priority)?;
    }
    Ok(lp)
}

#[cfg(test)]
#[path = "loop_methods_tests.rs"]
mod tests;
