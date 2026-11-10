use super::super::*;
use crate::types::PyObjectType;

#[pymethods]
impl BACnetClient {
    /// Read a device's objects for the experimental Standard 223 export
    /// (#2436): its name, its Object_List, then one request per object of
    /// the selected types. `to_turtle()` on the result writes the Turtle.
    ///
    /// The device is looked up in the discovery table, as the
    /// `_from_device` methods look it up. `types` narrows the objects read
    /// (analog, binary and multi-state inputs, outputs and values; all
    /// nine by default). Another type or an empty list raises ValueError
    /// before anything is sent. A failure before the Object_List is in
    /// raises the usual error; a device that stops answering after it
    /// gives an inventory with `stopped` set.
    #[pyo3(signature = (device_instance, *, types=None))]
    fn export_223_experimental<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        types: Option<Vec<PyObjectType>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = crate::export_223::options(types)?;
        let inner = self.inner.clone();

        crate::py_async::future_into_py(py, async move {
            let c = started(&inner).await?;
            crate::export_223::read(&c, device_instance, &options).await
        })
    }
}
