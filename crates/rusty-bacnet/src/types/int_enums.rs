//! `EngineeringUnits`, `ErrorClass`, `ErrorCode`, `RejectReason` and
//! `AbortReason` as Python `enum.IntEnum` classes (#1816).
//!
//! These five travel as plain integers in much of the API (the `units=`
//! arguments, a raised error's `error_class`, `error_code` and `reason`), so
//! their members must be `int`s: a member compares equal to its number, hashes
//! like it and passes wherever an `int` does. The other enumerations stay
//! frozen classes (see `enums.rs`).
//!
//! A value without a name (a proprietary code, or one the stack doesn't know)
//! is a pseudo-member of its class: `ErrorCode(300)` or `ErrorCode.from_raw(300)`
//! returns an `ErrorCode` that equals 300, has `name` `None` and reads as
//! `ErrorCode(300)`. So every value the stack hands out is an instance of its
//! class, named or not. A number outside the field's range is a `ValueError`.
//!
//! The members come from `ALL_NAMED` in `bacnet-types`, so nothing is listed
//! twice. Each class is built on first use, by the module's init or by the
//! first conversion, whichever comes first.

use std::ffi::CString;

use bacnet_types::enums as bacnet_enums;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBool, PyDict, PyList, PyType};

/// The shared base class: the pseudo-members, `from_raw`, `to_raw` and the
/// repr the frozen enum classes use (`ErrorClass.PROPERTY`).
const BASE: &str = r#"
import enum


class BacnetIntEnum(enum.IntEnum):
    """Base of the rusty_bacnet IntEnum classes."""

    @classmethod
    def _missing_(cls, value):
        if isinstance(value, bool) or not isinstance(value, int):
            return None
        if not 0 <= value <= cls._raw_max:
            return None
        member = int.__new__(cls, value)
        member._name_ = None
        member._value_ = int(value)
        return member

    @classmethod
    def from_raw(cls, value):
        """The member for ``value``; a pseudo-member when it has no name."""
        return cls(value)

    def to_raw(self):
        """The raw integer value."""
        return int(self)

    def __repr__(self):
        if self._name_ is None:
            return f"{type(self).__name__}({int(self)})"
        return f"{type(self).__name__}.{self._name_}"
"#;

static BASE_CLASS: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// One of the IntEnum classes.
pub(crate) trait PyIntEnum: Copy {
    /// The class name in `rusty_bacnet`.
    const NAME: &'static str;
    /// The class docstring.
    const DOC: &'static str;
    /// The largest raw value the field holds.
    const RAW_MAX: u32;

    /// Where the built class is kept.
    fn cell() -> &'static PyOnceLock<Py<PyType>>;
    /// The named values, in `ALL_NAMED` order.
    fn named() -> Vec<(&'static str, u32)>;
    /// The raw value.
    fn raw(self) -> u32;
    /// The value for a raw number within `RAW_MAX`.
    fn from_raw_checked(raw: u32) -> Self;
}

macro_rules! py_int_enum {
    ($Rust:ident, $raw:ty, $doc:literal) => {
        impl PyIntEnum for bacnet_enums::$Rust {
            const NAME: &'static str = stringify!($Rust);
            const DOC: &'static str = $doc;
            const RAW_MAX: u32 = <$raw>::MAX as u32;

            fn cell() -> &'static PyOnceLock<Py<PyType>> {
                static CELL: PyOnceLock<Py<PyType>> = PyOnceLock::new();
                &CELL
            }

            fn named() -> Vec<(&'static str, u32)> {
                Self::ALL_NAMED
                    .iter()
                    .map(|(name, value)| (*name, u32::from(value.to_raw())))
                    .collect()
            }

            fn raw(self) -> u32 {
                u32::from(self.to_raw())
            }

            fn from_raw_checked(raw: u32) -> Self {
                // RAW_MAX bounds `raw`, so the cast keeps every bit.
                Self::from_raw(raw as $raw)
            }
        }
    };
}

py_int_enum!(
    EngineeringUnits,
    u32,
    "BACnet engineering units (Clause 21, BACnetEngineeringUnits). Members are ints."
);
py_int_enum!(
    ErrorClass,
    u16,
    "BACnet error class (Clause 18). Members are ints."
);
py_int_enum!(
    ErrorCode,
    u16,
    "BACnet error code (Clause 18). Members are ints."
);
py_int_enum!(
    RejectReason,
    u8,
    "BACnet reject reason (Clause 18.9). Members are ints."
);
py_int_enum!(
    AbortReason,
    u8,
    "BACnet abort reason (Clause 18.10). Members are ints."
);

fn base_class(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    BASE_CLASS
        .get_or_try_init(py, || {
            let code = CString::new(BASE).expect("no NUL in the base class source");
            let module = PyModule::from_code(
                py,
                &code,
                c"rusty_bacnet/_int_enum.py",
                c"rusty_bacnet._int_enum",
            )?;
            Ok(module
                .getattr("BacnetIntEnum")?
                .cast_into::<PyType>()?
                .unbind())
        })
        .map(|class| class.bind(py))
}

/// `T`'s class, built on first use.
pub(crate) fn class<T: PyIntEnum>(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    T::cell()
        .get_or_try_init(py, || {
            let members = PyList::empty(py);
            for (name, value) in T::named() {
                members.append((name, value))?;
            }
            let options = PyDict::new(py);
            options.set_item("module", "rusty_bacnet")?;
            options.set_item("qualname", T::NAME)?;
            let class = base_class(py)?
                .call((T::NAME, members), Some(&options))?
                .cast_into::<PyType>()?;
            class.setattr("_raw_max", T::RAW_MAX)?;
            class.setattr("__doc__", T::DOC)?;
            Ok(class.unbind())
        })
        .map(|class| class.bind(py))
}

/// `value` as a member of its class: the named member, or a pseudo-member
/// for a value without a name.
pub(crate) fn to_py<'py, T: PyIntEnum>(py: Python<'py>, value: T) -> PyResult<Bound<'py, PyAny>> {
    class::<T>(py)?.call1((value.raw(),))
}

/// A member of `T`'s class or a plain int in its range, as the value. A
/// bool is refused, though Python counts it as an int.
pub(crate) fn from_py<T: PyIntEnum>(value: &Bound<'_, PyAny>) -> PyResult<T> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "expected {} or int, not bool",
            T::NAME
        )));
    }
    let raw: i128 = value
        .extract()
        .map_err(|_| PyTypeError::new_err(format!("expected {} or int", T::NAME)))?;
    match u32::try_from(raw) {
        Ok(raw) if raw <= T::RAW_MAX => Ok(T::from_raw_checked(raw)),
        _ => Err(PyValueError::new_err(format!(
            "{raw} is outside {}'s range 0..={}",
            T::NAME,
            T::RAW_MAX
        ))),
    }
}

/// A value converted to its IntEnum member when it goes to Python.
#[derive(Clone, Copy)]
pub(crate) struct IntEnumValue<T>(pub(crate) T);

impl<'py, T: PyIntEnum> IntoPyObject<'py> for IntEnumValue<T> {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
        to_py(py, self.0)
    }
}

/// Add the five classes to the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add(
        bacnet_enums::EngineeringUnits::NAME,
        class::<bacnet_enums::EngineeringUnits>(py)?,
    )?;
    m.add(
        bacnet_enums::ErrorClass::NAME,
        class::<bacnet_enums::ErrorClass>(py)?,
    )?;
    m.add(
        bacnet_enums::ErrorCode::NAME,
        class::<bacnet_enums::ErrorCode>(py)?,
    )?;
    m.add(
        bacnet_enums::RejectReason::NAME,
        class::<bacnet_enums::RejectReason>(py)?,
    )?;
    m.add(
        bacnet_enums::AbortReason::NAME,
        class::<bacnet_enums::AbortReason>(py)?,
    )?;
    Ok(())
}
