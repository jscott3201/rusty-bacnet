use super::*;

// ---------------------------------------------------------------------------
// Macro: generates a frozen pyclass wrapper for a `bacnet_enum!` newtype.
//
// Constants are registered dynamically from `ALL_NAMED` during module init,
// so there is zero constant duplication between bacnet-types and rusty-bacnet.
// ---------------------------------------------------------------------------

macro_rules! py_bacnet_enum {
    ($py_name:literal, $PyStruct:ident, $RustType:ty, $raw_ty:ty) => {
        #[pyclass(name = $py_name, frozen, from_py_object)]
        #[derive(Clone)]
        pub struct $PyStruct {
            pub(crate) inner: $RustType,
        }

        impl $PyStruct {
            pub fn to_rust(&self) -> $RustType {
                self.inner
            }

            /// Set every named constant as a class attribute (e.g. `ObjectType.DEVICE`).
            pub fn register_constants(cls: &Bound<'_, PyAny>) -> PyResult<()> {
                for &(name, val) in <$RustType>::ALL_NAMED {
                    cls.setattr(name, Self { inner: val })?;
                }
                Ok(())
            }
        }

        #[pymethods]
        impl $PyStruct {
            /// Create from a raw integer value.
            #[staticmethod]
            fn from_raw(value: $raw_ty) -> Self {
                Self {
                    inner: <$RustType>::from_raw(value),
                }
            }

            /// Get the raw integer value.
            fn to_raw(&self) -> $raw_ty {
                self.inner.to_raw()
            }

            fn __repr__(&self) -> String {
                format!(concat!($py_name, ".{}"), self.inner)
            }

            fn __eq__(&self, other: &Self) -> bool {
                self.inner == other.inner
            }

            fn __hash__(&self) -> u64 {
                self.inner.to_raw() as u64
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Enum wrappers — constants are populated at module init time via ALL_NAMED.
// ---------------------------------------------------------------------------

py_bacnet_enum!("ObjectType", PyObjectType, bacnet_enums::ObjectType, u32);
py_bacnet_enum!(
    "PropertyIdentifier",
    PyPropertyIdentifier,
    bacnet_enums::PropertyIdentifier,
    u32
);
py_bacnet_enum!("ErrorClass", PyErrorClass, bacnet_enums::ErrorClass, u16);
py_bacnet_enum!("ErrorCode", PyErrorCode, bacnet_enums::ErrorCode, u16);
py_bacnet_enum!(
    "EnableDisable",
    PyEnableDisable,
    bacnet_enums::EnableDisable,
    u32
);
py_bacnet_enum!(
    "ReinitializedState",
    PyReinitializedState,
    bacnet_enums::ReinitializedState,
    u32
);
py_bacnet_enum!(
    "Segmentation",
    PySegmentation,
    bacnet_enums::Segmentation,
    u8
);
py_bacnet_enum!(
    "LifeSafetyOperation",
    PyLifeSafetyOperation,
    bacnet_enums::LifeSafetyOperation,
    u32
);
py_bacnet_enum!("EventState", PyEventState, bacnet_enums::EventState, u32);
py_bacnet_enum!("EventType", PyEventType, bacnet_enums::EventType, u32);
py_bacnet_enum!(
    "MessagePriority",
    PyMessagePriority,
    bacnet_enums::MessagePriority,
    u32
);
