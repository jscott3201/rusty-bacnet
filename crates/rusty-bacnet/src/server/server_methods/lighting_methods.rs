//! Lighting outputs and colour objects (#1554): the trims (#1528), the
//! colour links (#1527), the Color and Color Temperature objects, the colour
//! an output shows, and the profile and Value_Source keywords (#1552, #1553).
use super::super::*;
use crate::object_options::provision_profile;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject, FileColorOutputPersistence};
use bacnet_objects::lighting::{ColorLink, ColorOverride, OutputColor};
use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::error::Error;
use pyo3::exceptions::PyValueError;
use pyo3::IntoPyObjectExt;

#[pymethods]
impl BACnetServer {
    /// Add a Lighting Output object to the server (before starting).
    ///
    /// Its Lighting_Command reads and writes as `application_data` holding
    /// the context-tagged BACnetLightingCommand, operation NONE until written
    /// (#1263). The object carries each command out (#1384): fades and ramps
    /// move Tracking_Value to the new level over time, steps change the level
    /// at once, and the warn commands blink and hold the level for
    /// Egress_Time when Blink_Warn_Enable is TRUE.
    ///
    /// A Present_Value or Relinquish_Default level above 0.0 and below 1.0 is
    /// stored as 1.0, and one outside 0.0 to 100.0 is refused with
    /// VALUE_OUT_OF_RANGE (#1385), but for Present_Value's warn values -1.0,
    /// -2.0 and -3.0. Tracking_Value reads the same level as Present_Value
    /// whenever no fade or ramp is running.
    ///
    /// `high_end_trim` and `low_end_trim` (1.0 to 100.0 percent) serve the
    /// trims that bound Tracking_Value, and with either one Trim_Fade_Time,
    /// which `trim_fade_time` sets in milliseconds (0 to 86,400,000). The
    /// colour keywords are as on `add_binary_lighting_output`. A value out of
    /// range raises BacnetProtocolError (VALUE_OUT_OF_RANGE).
    #[pyo3(signature = (
        instance, name, *,
        high_end_trim=None, low_end_trim=None, trim_fade_time=None,
        color_reference=None, color_override=None, override_color_reference=None,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_lighting_output(
        &self,
        instance: u32,
        name: &str,
        high_end_trim: Option<f32>,
        low_end_trim: Option<f32>,
        trim_fade_time: Option<u32>,
        color_reference: Option<PyObjectIdentifier>,
        color_override: Option<bool>,
        override_color_reference: Option<PyObjectIdentifier>,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let link = color_link(color_reference, color_override, override_color_reference)?;
        let mut obj = LightingOutputObject::new(instance, name).map_err(to_py_err)?;
        let trims = (|| -> Result<(), Error> {
            if let Some(milliseconds) = trim_fade_time {
                obj.set_trim_fade_time(milliseconds)?;
            }
            if high_end_trim.is_some() {
                obj.set_high_end_trim(high_end_trim)?;
            }
            if low_end_trim.is_some() {
                obj.set_low_end_trim(low_end_trim)?;
            }
            obj.set_color_link(link)
        })();
        trims.map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| obj.set_profile(p))?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Binary Lighting Output object to the server (before starting).
    ///
    /// `color_reference` serves Color_Reference, the Color or Color
    /// Temperature object the output takes its colour from (instance 4194303
    /// names none). `override_color_reference` adds Color_Override and
    /// Override_Color_Reference; `color_override=True` sets the override on,
    /// and needs it (False, the default, means no override). A reference to another object type raises
    /// BacnetProtocolError (VALUE_OUT_OF_RANGE); `lighting_color` follows the
    /// references once the server runs.
    #[pyo3(signature = (
        instance, name, *,
        color_reference=None, color_override=None, override_color_reference=None,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_binary_lighting_output(
        &self,
        instance: u32,
        name: &str,
        color_reference: Option<PyObjectIdentifier>,
        color_override: Option<bool>,
        override_color_reference: Option<PyObjectIdentifier>,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let link = color_link(color_reference, color_override, override_color_reference)?;
        let mut obj = BinaryLightingOutputObject::new(instance, name).map_err(to_py_err)?;
        obj.set_color_link(link).map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| obj.set_profile(p))?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Color object to the server (before starting): a CIE 1931 xy
    /// colour, D65 white (0.3127, 0.3290) unless `present_value` gives an
    /// `(x, y)` pair, each 0.0 to 1.0 (BacnetProtocolError otherwise).
    ///
    /// `value_source_tracking` serves Value_Source, the writer of the last
    /// Present_Value or Color_Command that set it (#1552).
    ///
    /// At start the server drives the output to `default_color` (D65 unless
    /// given), at once (#1526), over any `present_value`. `(0.0, 0.0)`
    /// instead keeps the colour from before the restart, kept in
    /// `storage_path` (a `str`, the file the object keeps Present_Value in);
    /// with nothing stored it keeps `present_value`, and with neither,
    /// In_Progress reads NOT_CONTROLLED until a write drives the output.
    #[pyo3(signature = (
        instance, name, *, present_value=None, default_color=None, storage_path=None,
        value_source_tracking=false, tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_color(
        &self,
        instance: u32,
        name: &str,
        present_value: Option<(f32, f32)>,
        default_color: Option<(f32, f32)>,
        storage_path: Option<&str>,
        value_source_tracking: bool,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let output = ColorOutput {
            present_value: present_value.map(|(x, y)| BACnetXyColor::new(x, y)),
            default: default_color.map(|(x, y)| BACnetXyColor::new(x, y)),
            storage_path,
        };
        let mut obj = color(instance, name, output).map_err(to_py_err)?;
        obj.set_value_source_tracking(value_source_tracking);
        provision_profile(tags, profile_location, profile_name, |p| obj.set_profile(p))?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Color Temperature object to the server (before starting): a
    /// correlated colour temperature in kelvin, 4000 unless `present_value`
    /// says otherwise.
    ///
    /// `min_pres_value` and `max_pres_value` narrow the 1000 to 30000 K range
    /// (both or neither); `present_value` is clamped to them. A value out of
    /// range raises BacnetProtocolError. `value_source_tracking` is as on
    /// `add_color`.
    ///
    /// `default_color_temperature` (4000 unless given, clamped to the
    /// limits) and `storage_path` work as `default_color` and `storage_path`
    /// on `add_color`, with 0 keeping the temperature from before the
    /// restart (#1526).
    #[pyo3(signature = (
        instance, name, *, present_value=None, min_pres_value=None, max_pres_value=None,
        default_color_temperature=None, storage_path=None, value_source_tracking=false,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_color_temperature(
        &self,
        instance: u32,
        name: &str,
        present_value: Option<u32>,
        min_pres_value: Option<u32>,
        max_pres_value: Option<u32>,
        default_color_temperature: Option<u32>,
        storage_path: Option<&str>,
        value_source_tracking: bool,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let limits = match (min_pres_value, max_pres_value) {
            (Some(min), Some(max)) => Some((min, max)),
            (None, None) => None,
            _ => {
                return Err(PyValueError::new_err(
                    "give min_pres_value and max_pres_value together",
                ))
            }
        };
        let output = ColorOutput {
            present_value,
            default: default_color_temperature,
            storage_path,
        };
        let mut obj = color_temperature(instance, name, limits, output).map_err(to_py_err)?;
        obj.set_value_source_tracking(value_source_tracking);
        provision_profile(tags, profile_location, profile_name, |p| obj.set_profile(p))?;
        self.push_pending(Box::new(obj))
    }

    /// The colour the lighting output `object_id` shows now (#1527): the
    /// Tracking_Value of the colour object its Color_Reference names, or its
    /// Override_Color_Reference while Color_Override is TRUE, so a colour
    /// fade shows as it runs.
    ///
    /// Returns `(source, colour)`: the colour object's identifier and an
    /// `(x, y)` pair for a Color object or an int in kelvin for a Color
    /// Temperature object. `None` when the output has no Color_Reference,
    /// the reference in use names no object (instance 4194303), the server
    /// holds no colour object by that identifier, or that object reads
    /// In_Progress NOT_CONTROLLED (#1526).
    fn lighting_color<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        crate::py_async::future_into_py(py, async move {
            // Take the database handle and let go of the server first, so
            // the read holds one lock.
            let db = {
                let guard = inner.lock().await;
                let srv = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                srv.database().clone()
            };
            let color = db.read().await.lighting_color(&oid);
            Ok(color.map(|color| LightingColorValue {
                source: PyObjectIdentifier::from_rust(color.source),
                color: color.color,
            }))
        })
    }
}

/// A [`bacnet_objects::lighting::LightingColor`] on its way to Python.
struct LightingColorValue {
    source: PyObjectIdentifier,
    color: OutputColor,
}

impl<'py> IntoPyObject<'py> for LightingColorValue {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let color = match self.color {
            OutputColor::Xy(xy) => (f64::from(xy.x), f64::from(xy.y)).into_py_any(py)?,
            OutputColor::Kelvin(kelvin) => kelvin.into_py_any(py)?,
        };
        (self.source, color).into_pyobject(py).map(Bound::into_any)
    }
}

/// The colour link the keywords describe, or `None` without a
/// `color_reference`. An override reference, or `color_override=True`,
/// needs one, and `color_override=True` also needs `override_color_reference`
/// (ValueError otherwise); `color_override=False` alone means no override.
/// The object checks each reference's type.
fn color_link(
    reference: Option<PyObjectIdentifier>,
    active: Option<bool>,
    override_reference: Option<PyObjectIdentifier>,
) -> PyResult<Option<ColorLink>> {
    let Some(reference) = reference else {
        if active == Some(true) || override_reference.is_some() {
            return Err(PyValueError::new_err(
                "color_override and override_color_reference need a color_reference",
            ));
        }
        return Ok(None);
    };
    let color_override = match (override_reference, active) {
        (Some(reference), active) => Some(ColorOverride {
            active: active.unwrap_or(false),
            reference: reference.to_rust(),
        }),
        (None, Some(true)) => {
            return Err(PyValueError::new_err(
                "color_override=True needs an override_color_reference",
            ))
        }
        (None, Some(false) | None) => None,
    };
    Ok(Some(ColorLink {
        reference: reference.to_rust(),
        color_override,
    }))
}

/// The output keywords of `add_color` (`V` an xy colour) and
/// `add_color_temperature` (`V` kelvin).
struct ColorOutput<'a, V> {
    present_value: Option<V>,
    default: Option<V>,
    storage_path: Option<&'a str>,
}

impl<V> ColorOutput<'_, V> {
    /// The storage for `storage_path`, if given.
    fn storage(&self) -> Result<Option<Arc<FileColorOutputPersistence>>, Error> {
        self.storage_path
            .map(|path| FileColorOutputPersistence::new(path).map(Arc::new))
            .transpose()
    }
}

/// Build a Color object: kept in the file at `storage_path` when one is
/// given (#1526), then its default and Present_Value set through their
/// checking setters. A colour storage brought back wins over
/// `present_value`, as a write it saved would.
fn color(
    instance: u32,
    name: &str,
    output: ColorOutput<'_, BACnetXyColor>,
) -> Result<ColorObject, Error> {
    let mut obj = ColorObject::new(instance, name)?;
    if let Some(storage) = output.storage()? {
        obj = obj.with_output_persistence(storage)?;
    }
    if let Some(color) = output.default {
        obj.set_default_color(color)?;
    }
    if let Some(color) = output.present_value.filter(|_| !obj.output_restored()) {
        obj.set_present_value(color)?;
    }
    Ok(obj)
}

/// Build a Color Temperature object as [`color`] builds a Color object,
/// with `limits` (Min_Pres_Value, Max_Pres_Value) set before the default
/// and Present_Value so both are clamped to them.
fn color_temperature(
    instance: u32,
    name: &str,
    limits: Option<(u32, u32)>,
    output: ColorOutput<'_, u32>,
) -> Result<ColorTemperatureObject, Error> {
    let mut obj = ColorTemperatureObject::new(instance, name)?;
    if let Some(storage) = output.storage()? {
        obj = obj.with_output_persistence(storage)?;
    }
    if let Some((min, max)) = limits {
        obj.set_min_max(min, max)?;
    }
    if let Some(kelvin) = output.default {
        obj.set_default_color_temperature(kelvin)?;
    }
    if let Some(kelvin) = output.present_value.filter(|_| !obj.output_restored()) {
        obj.set_present_value(kelvin)?;
    }
    Ok(obj)
}

#[cfg(test)]
#[path = "lighting_methods_tests.rs"]
mod tests;
