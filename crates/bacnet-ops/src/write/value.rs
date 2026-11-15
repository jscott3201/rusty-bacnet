//! What a write sends: a value typed for its property ([`IntoWriteValue`]).

use bacnet_types::primitives::PropertyValue;

use super::Priority;
use crate::error::OpsError;
use crate::parse::parse_value_for_with_priority;
use crate::reading::PropertyRef;

/// A value to write, typed for its property, with the priority its text
/// names after an `@` (`72.5@8`), if any.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteValue {
    /// The value, as it goes on the wire.
    pub value: PropertyValue,
    /// The priority the text names; `None` for a typed value.
    pub priority: Option<Priority>,
}

/// What names the value of a [`Device::write`](crate::Device::write): text
/// in the value grammar, read against the property's datatype, or a typed
/// [`PropertyValue`], sent as it is.
///
/// Text is read as [`parse_value_for`](crate::parse::parse_value_for)
/// reads it for the property written: `72.5` is a REAL for an analog
/// object's Present_Value and a Double for a Large Analog Value's, `active`
/// a binary state, `in-alarm,fault` a Status_Flags, and so on. It may end
/// with `@` and a priority (`72.5@8`).
pub trait IntoWriteValue {
    /// The value for a write of `target`.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] (`what` is `"value"`) for text that doesn't read
    /// as a value of the property, or names a priority outside 1 to 16.
    fn into_write_value(self, target: &PropertyRef) -> Result<WriteValue, OpsError>;
}

impl IntoWriteValue for &str {
    fn into_write_value(self, target: &PropertyRef) -> Result<WriteValue, OpsError> {
        let object_type = target.object.object_type();
        let (value, priority) =
            parse_value_for_with_priority(object_type, target.property, target.index, self)
                .map_err(|reason| OpsError::parse("value", self, reason))?;
        Ok(WriteValue { value, priority })
    }
}

impl IntoWriteValue for &String {
    fn into_write_value(self, target: &PropertyRef) -> Result<WriteValue, OpsError> {
        self.as_str().into_write_value(target)
    }
}

impl IntoWriteValue for String {
    fn into_write_value(self, target: &PropertyRef) -> Result<WriteValue, OpsError> {
        self.as_str().into_write_value(target)
    }
}

impl IntoWriteValue for PropertyValue {
    fn into_write_value(self, _: &PropertyRef) -> Result<WriteValue, OpsError> {
        Ok(WriteValue {
            value: self,
            priority: None,
        })
    }
}

impl IntoWriteValue for WriteValue {
    fn into_write_value(self, _: &PropertyRef) -> Result<WriteValue, OpsError> {
        Ok(self)
    }
}
