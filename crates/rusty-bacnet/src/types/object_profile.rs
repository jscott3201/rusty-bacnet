//! The `tags`, `profile_location` and `profile_name` keywords of the object
//! builders (#1553, #1554): the optional Tags, Profile_Location and
//! Profile_Name rows, provisioned before the object is registered.
//!
//! `tags` takes the form a typed Tags read gives as `.value`: a list of
//! `{"name": str, "value": PropertyValue | None}` mappings, `value` optional.
//! This layer checks shapes and Python types; the object's `set_profile`
//! checks the rest (a semicolon in a name, a value that isn't primitive, a
//! location's URI scheme, a profile name's vendor prefix) and refuses with a
//! protocol error.

use bacnet_objects::object_profile::ObjectProfile;
use bacnet_types::constructed::BACnetNameValue;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use super::mapping::{mapping, optional_item, required_item, string, validate_keys};
use super::PyPropertyValue;

/// The rows the three keywords provision, or `None` when all three are
/// absent, so that an object built without them serves none of the rows.
pub(crate) fn object_profile_from_py(
    tags: Option<&Bound<'_, PyAny>>,
    profile_location: Option<String>,
    profile_name: Option<String>,
) -> PyResult<Option<ObjectProfile>> {
    if tags.is_none() && profile_location.is_none() && profile_name.is_none() {
        return Ok(None);
    }
    let tags = tags.map(name_values).transpose()?;
    Ok(Some(ObjectProfile {
        tags,
        profile_location,
        profile_name,
    }))
}

/// A list of tag mappings, in order.
fn name_values(value: &Bound<'_, PyAny>) -> PyResult<Vec<BACnetNameValue>> {
    let items: Vec<Bound<'_, PyAny>> = value
        .extract()
        .map_err(|_| PyTypeError::new_err("tags must be a list"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| name_value(item, &format!("tags[{index}]")))
        .collect()
}

fn name_value(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetNameValue> {
    let value = mapping(value, name)?;
    validate_keys(value, name, &["name"], &["value"])?;
    let tag_name = string(
        &required_item(value, name, "name")?,
        &format!("{name}.name"),
    )?;
    let tag_value = optional_item(value, "value")?
        .map(|item| {
            item.extract::<PyPropertyValue>()
                .map(|value| value.inner)
                .map_err(|_| {
                    PyTypeError::new_err(format!("{name}.value must be a PropertyValue or None"))
                })
        })
        .transpose()?;
    Ok(BACnetNameValue {
        name: tag_name,
        value: tag_value,
    })
}
