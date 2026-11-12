//! A Load Control's shed level in Python: the mapping a typed read of
//! Requested_Shed_Level gives (#1310), `{"kind": "percent" or "level",
//! "value": int}` or `{"kind": "amount", "value": float}`, which a Python
//! shedder also answers with (#2479).

use bacnet_types::constructed::BACnetShedLevel;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt};

use super::mapping::{discriminator, fixed_integer, mapping, required_item, validate_keys};

/// `level` as the mapping of its choice and value.
pub(crate) fn shed_level_to_py<'py>(
    py: Python<'py>,
    level: &BACnetShedLevel,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    match *level {
        BACnetShedLevel::Percent(percent) => {
            dict.set_item("kind", "percent")?;
            dict.set_item("value", percent)?;
        }
        BACnetShedLevel::Level(level) => {
            dict.set_item("kind", "level")?;
            dict.set_item("value", level)?;
        }
        BACnetShedLevel::Amount(amount) => {
            dict.set_item("kind", "amount")?;
            dict.set_item("value", f64::from(amount))?;
        }
    }
    Ok(dict)
}

/// The shed level a mapping of `kind` and `value` names. A percent or a level
/// is an Unsigned (an `int` from 0, OverflowError outside 64 bits); an amount
/// is a `float` or an `int` that is finite as a REAL. Any other shape raises
/// TypeError, and an unknown kind or key ValueError, naming `name`. The range
/// a request allows is the object's to hold.
pub(crate) fn shed_level_from_py(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetShedLevel> {
    let value = mapping(value, name)?;
    validate_keys(value, name, &["kind", "value"], &[])?;
    let kind = discriminator(value, name)?;
    let item = required_item(value, name, "value")?;
    let field = format!("{name}.value");
    match kind.as_str() {
        "percent" => Ok(BACnetShedLevel::Percent(fixed_integer(&item, &field)?)),
        "level" => Ok(BACnetShedLevel::Level(fixed_integer(&item, &field)?)),
        "amount" => {
            let number = item.is_instance_of::<PyFloat>()
                || (item.is_instance_of::<PyInt>() && !item.is_instance_of::<PyBool>());
            if !number {
                return Err(PyTypeError::new_err(format!("{field} must be a float")));
            }
            let amount = item.extract::<f64>()? as f32;
            if !amount.is_finite() {
                return Err(PyValueError::new_err(format!(
                    "{field} must be finite as a single-precision REAL"
                )));
            }
            Ok(BACnetShedLevel::Amount(amount))
        }
        _ => Err(PyValueError::new_err(format!(
            "{name}.kind must be 'percent', 'level' or 'amount', got '{kind}'"
        ))),
    }
}
