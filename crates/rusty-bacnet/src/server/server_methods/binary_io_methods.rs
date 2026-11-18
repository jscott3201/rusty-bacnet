//! The application's routes to the physical side of a running Binary Input
//! and Binary Output and to their Polarity (#2580), over the Rust server's
//! `set_physical_input_local`, `set_polarity_local` and
//! `physical_output_local`.
use super::super::*;
use bacnet_types::enums::Polarity;

#[pymethods]
impl BACnetServer {
    /// Report the state the application read from a Binary Input's physical
    /// input, `True` for ON.
    ///
    /// In service Present_Value becomes that state through Polarity: ACTIVE
    /// for ON under NORMAL (0), INACTIVE for ON under REVERSE (1). A logical
    /// value goes through `set_present_value_local` instead. This route is
    /// taken while Out_Of_Service is set too: Present_Value then keeps a
    /// client's simulated value, and the return to service derives it from
    /// the latest input. Event and COV processing see the logical
    /// Present_Value. An unknown object raises UNKNOWN_OBJECT and any object
    /// other than a Binary Input OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    #[pyo3(signature = (object_id, on))]
    fn set_physical_input_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        on: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let future = async move {
            let guard = inner.read().await;
            let srv = guard.server()?;
            srv.set_physical_input_local(&oid, on)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Set the Polarity of a Binary Input or Binary Output: a `Polarity`
    /// or its number, NORMAL (0) or REVERSE (1). Clients can't write it.
    ///
    /// In service a Binary Input derives Present_Value again from the
    /// physical input it last took, and a Binary Output drives its physical
    /// output from its unchanged Present_Value with the new Polarity; out
    /// of service neither changes until the return to service. Any other
    /// int, negative included, raises VALUE_OUT_OF_RANGE, an unknown object
    /// UNKNOWN_OBJECT and any other object
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    #[pyo3(signature = (object_id, polarity))]
    fn set_polarity_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        #[pyo3(from_py_with = crate::types::int_enums::polarity_arg)] polarity: u32,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let polarity = Polarity::from_raw(polarity);
        let future = async move {
            let guard = inner.read().await;
            let srv = guard.server()?;
            srv.set_polarity_local(&oid, polarity)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// The state to drive a Binary Output's physical output to, `True` for
    /// ON.
    ///
    /// In service it is the commanded Present_Value through Polarity: ON
    /// for ACTIVE under NORMAL, OFF for ACTIVE under REVERSE. While
    /// Out_Of_Service is set it stays at the state it had when
    /// Out_Of_Service was set, though commands keep moving Present_Value.
    /// An unknown object raises UNKNOWN_OBJECT and any object other than a
    /// Binary Output OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    #[pyo3(signature = (object_id))]
    fn physical_output_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        crate::py_async::future_into_py(py, async move {
            let guard = inner.read().await;
            let srv = guard.server()?;
            srv.physical_output_local(&oid).await.map_err(to_py_err)
        })
    }
}
