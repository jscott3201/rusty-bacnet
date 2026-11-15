//! [`PyWritePreview`]: what a write would do, read before it is sent
//! (`Device.preview_write`, #2561).

use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::target::AddressStyle;
use bacnet_ops::write::{Effect, Occupancy};
use bacnet_ops::{Priority, PropertyRef, WritePreview};
use pyo3::prelude::*;

use super::errors::write_denied;
use super::priorities::PyPriorityView;
use super::value::PyValue;
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

/// What `Device.write` of the same arguments would do, read first and sent
/// nowhere. Refuses `copy` and `pickle`.
#[pyclass(name = "WritePreview", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWritePreview {
    address: String,
    property: PropertyRef,
    value: Py<PyValue>,
    priority: Option<Priority>,
    occupancy: Occupancy,
    before: Option<Py<PyValue>>,
    effect: Effect,
    priorities: Option<Py<PyPriorityView>>,
    /// The BacnetWriteDeniedError the write would raise, unraised.
    denied: Option<Py<PyAny>>,
}

impl PyWritePreview {
    /// `preview` as Python objects, its address as a link of `style`
    /// writes it.
    pub(crate) fn new(
        py: Python<'_>,
        preview: &WritePreview,
        style: AddressStyle,
    ) -> PyResult<Self> {
        let property = preview.property;
        let value = |value| {
            Py::new(
                py,
                PyValue::new(
                    property.object.object_type(),
                    property.property,
                    property.index,
                    value,
                ),
            )
        };
        Ok(Self {
            address: preview.device.styled(style),
            property,
            value: value(&preview.value)?,
            priority: preview.priority,
            occupancy: preview.occupancy,
            before: preview.before.as_ref().map(value).transpose()?,
            effect: preview.effect.clone(),
            priorities: preview
                .priorities
                .as_ref()
                .map(|view| Py::new(py, PyPriorityView::new(py, view)?))
                .transpose()?,
            denied: preview
                .denied
                .as_ref()
                .map(|denied| write_denied(denied).into_value(py).into_any()),
        })
    }
}

#[pymethods]
impl PyWritePreview {
    /// Where the read went, and the write would go, as `Device.address`
    /// writes it.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The object it would write.
    #[getter]
    fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.property.object)
    }

    /// The property it would write.
    #[getter]
    fn property_id(&self) -> PyPropertyIdentifier {
        PyPropertyIdentifier {
            inner: self.property.property,
        }
    }

    /// The array index it would write, None for the whole property.
    #[getter]
    fn array_index(&self) -> Option<u32> {
        self.property.index
    }

    /// The value as it would be sent, a `Value`.
    #[getter]
    fn value(&self, py: Python<'_>) -> Py<PyValue> {
        self.value.clone_ref(py)
    }

    /// The slot it would take: the priority asked for, or 16, on a
    /// property the read found commandable; None on one it found isn't.
    #[getter]
    fn priority(&self) -> Option<u8> {
        self.priority.map(Priority::get)
    }

    /// Who holds that slot now: `"empty"`, `"held"` (the write replaces a
    /// command, without telling whoever sent it), `"not-commandable"` or
    /// `"unknown"` (the device wouldn't give its slots).
    #[getter]
    fn occupancy(&self) -> &'static str {
        match self.occupancy {
            Occupancy::NotCommandable => "not-commandable",
            Occupancy::Empty => "empty",
            Occupancy::Held => "held",
            _ => "unknown",
        }
    }

    /// What the write replaces, a `Value`: the slot's command when one
    /// holds it, or the property's value for one that isn't commandable.
    /// None for an empty slot, or when the device refused the read.
    #[getter]
    fn before(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.before.as_ref().map(|value| value.clone_ref(py))
    }

    /// What the write does to Present_Value: `"changes"` (see
    /// `changes_from` and `changes_to`), `"unchanged"` (a stronger slot
    /// keeps control when `held_by` names one), `"not-present-value"` for
    /// a write to another property, or `"unknown"`.
    #[getter]
    fn effect(&self) -> &'static str {
        match self.effect {
            Effect::Changes { .. } => "changes",
            Effect::Unchanged { .. } => "unchanged",
            Effect::NotPresentValue => "not-present-value",
            Effect::Unknown => "unknown",
        }
    }

    /// Present_Value's text now, when `effect` is `"changes"`; else None.
    #[getter]
    fn changes_from(&self) -> Option<&str> {
        match &self.effect {
            Effect::Changes { from, .. } => Some(from),
            _ => None,
        }
    }

    /// Present_Value's text after the write, when `effect` is `"changes"`;
    /// else None.
    #[getter]
    fn changes_to(&self) -> Option<&str> {
        match &self.effect {
            Effect::Changes { to, .. } => Some(to),
            _ => None,
        }
    }

    /// The stronger priority that keeps control, when `effect` is
    /// `"unchanged"` because of one; else None.
    #[getter]
    fn held_by(&self) -> Option<u8> {
        match self.effect {
            Effect::Unchanged { held_by } => held_by.map(Priority::get),
            _ => None,
        }
    }

    /// The object's commands as read, a `PriorityView`, for a commandable
    /// property; else None.
    #[getter]
    fn priorities(&self, py: Python<'_>) -> Option<Py<PyPriorityView>> {
        self.priorities.as_ref().map(|view| view.clone_ref(py))
    }

    /// The BacnetWriteDeniedError `Device.write` would raise, unraised,
    /// when the client's write policy refuses the priority; else None.
    #[getter]
    fn denied(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.denied.as_ref().map(|denied| denied.clone_ref(py))
    }

    fn __repr__(&self) -> String {
        let index = self
            .property
            .index
            .map(|i| format!("[{i}]"))
            .unwrap_or_default();
        let priority = self
            .priority
            .map(|p| format!(" at priority {p}"))
            .unwrap_or_default();
        let denied = if self.denied.is_some() {
            ", denied"
        } else {
            ""
        };
        format!(
            "WritePreview({} {}{index} = {}{priority}, {}, {}{denied})",
            object_label(self.property.object),
            kebab(self.property.property),
            self.value.get().text(),
            self.occupancy(),
            self.effect()
        )
    }

    /// Refuses `copy` and `pickle`, as the other received results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
