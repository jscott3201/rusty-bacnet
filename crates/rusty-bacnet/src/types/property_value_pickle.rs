//! Rebuilding PropertyValue for copying and pickling.

use super::*;
use pyo3::types::PyTuple;

#[pymethods]
impl PyPropertyValue {
    /// What `copy` and `pickle` call (#1500): the constructor `tag` names
    /// and its arguments, taken from the value as stored, so the copy is
    /// equal. A list passes its items as PropertyValues, keeping each one's
    /// tag, where `.value` would give `real` and `double` alike as a float.
    /// A typed constructed element (#1310) rebuilds from its octets as read
    /// through `_typed_element`: `.value` is decoded from those octets and
    /// needn't encode back to them. A value read from a property (#2080)
    /// rebuilds through `_read_from` with the value and that property, so the
    /// copy keeps its names.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        use primitives::PropertyValue as V;
        let py = slf.py();
        let this = slf.get();
        if let Some(context) = &this.context {
            let mut value = this.clone();
            value.context = None;
            let arguments = (
                value,
                context.object_id(),
                context.property_id,
                context.array_index,
                context.raw(py),
            )
                .into_pyobject(py)?;
            return Ok((slf.get_type().getattr("_with_read_context")?, arguments));
        }
        if let Some(property) = this.property {
            let mut value = this.clone();
            value.property = None;
            let arguments = (value, PyPropertyIdentifier { inner: property }).into_pyobject(py)?;
            return Ok((slf.get_type().getattr("_read_from")?, arguments));
        }
        if let (V::ApplicationData(octets), Form::NetworkPortTable(table)) =
            (&this.inner, this.form)
        {
            let arguments = (table.tag(), PyBytes::new(py, octets)).into_pyobject(py)?;
            return Ok((slf.get_type().getattr("_network_port_table")?, arguments));
        }
        if let (V::ApplicationData(octets), Form::Tagged) = (&this.inner, this.form) {
            let arguments = (PyBytes::new(py, octets),).into_pyobject(py)?;
            return Ok((slf.get_type().getattr(TAGGED)?, arguments));
        }
        let constructor = match (&this.inner, this.form) {
            (V::ApplicationData(_), Form::Typed(_)) => "_typed_element",
            (inner, _) => tag(inner),
        };
        let arguments = match (&this.inner, this.form) {
            (V::ApplicationData(octets), Form::Typed(element)) => {
                (element.tag(), PyBytes::new(py, octets)).into_pyobject(py)?
            }
            (V::List(items), form) => {
                // A list's application data items are read in its form.
                let items: Vec<Self> = items
                    .iter()
                    .map(|item| Self {
                        form: match item {
                            V::ApplicationData(_) => form,
                            _ => Form::Plain,
                        },
                        ..Self::from_rust(item.clone())
                    })
                    .collect();
                (items,).into_pyobject(py)?
            }
            (V::Null, _) => PyTuple::empty(py),
            (V::Boolean(value), _) => (*value,).into_pyobject(py)?,
            (V::Unsigned(value), _) => (*value,).into_pyobject(py)?,
            (V::Signed(value), _) => (*value,).into_pyobject(py)?,
            (V::Real(value), _) => (f64::from(*value),).into_pyobject(py)?,
            (V::Double(value), _) => (*value,).into_pyobject(py)?,
            (V::CharacterString(value), _) => (value.as_str(),).into_pyobject(py)?,
            (V::Enumerated(value), _) => (*value,).into_pyobject(py)?,
            (V::OctetString(octets) | V::ApplicationData(octets), _) => {
                (PyBytes::new(py, octets),).into_pyobject(py)?
            }
            (V::BitString { unused_bits, data }, _) => {
                (*unused_bits, PyBytes::new(py, data)).into_pyobject(py)?
            }
            (V::Date(date), _) => date_value(date).into_pyobject(py)?,
            (V::Time(time), _) => time_value(time).into_pyobject(py)?,
            (V::ObjectIdentifier(oid), _) => {
                (PyObjectIdentifier::from_rust(*oid),).into_pyobject(py)?
            }
        };
        Ok((slf.get_type().getattr(constructor)?, arguments))
    }

    /// `value` as read from `property`, so its enumerated value and bit
    /// string come out named: what `__reduce__` makes `copy` and `pickle`
    /// call for a value a read gave.
    #[staticmethod]
    fn _read_from(value: Self, property: PyPropertyIdentifier) -> Self {
        value.read_from(property.to_rust())
    }
}
