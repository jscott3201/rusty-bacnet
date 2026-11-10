//! A Color Temperature object's output storage (#1526) and its written
//! default (#1956); see [`persistence`](super::super::persistence).

use alloc::sync::Arc;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::super::command;
use super::super::persistence::{ColorOutputPersistence, ColorOutputValue};
use super::super::saving::ColorOutputSaveCounters;
use super::{ColorTemperatureObject, written_kelvin};
use crate::common;
use crate::durable::{PendingWrite, StageStep};

/// A written Default_Color_Temperature as storage keeps it: 0, or a
/// temperature clamped to `min..=max`.
fn written_default(value: &PropertyValue, min: u32, max: u32) -> Result<ColorOutputValue, Error> {
    match *value {
        PropertyValue::Unsigned(0) => Ok(ColorOutputValue::Kelvin(0)),
        PropertyValue::Unsigned(kelvin) => {
            written_kelvin(kelvin, min, max).map(ColorOutputValue::Kelvin)
        }
        _ => Err(common::invalid_data_type_error()),
    }
}

impl ColorTemperatureObject {
    /// Keep Present_Value and a written Default_Color_Temperature in
    /// `persistence` (#1526, #1956), and take what it saved for this object,
    /// if anything: the temperature as Present_Value, clamped to
    /// Min_Pres_Value and Max_Pres_Value, and a default a write set as
    /// Default_Color_Temperature. Call it right after building the object,
    /// before anything writes Present_Value.
    ///
    /// Saving works as on a Color object
    /// ([`ColorObject::with_output_persistence`](super::super::ColorObject::with_output_persistence)).
    /// With a Default_Color_Temperature of 0, the device's start keeps the
    /// saved temperature (Clause 12.Y.4).
    ///
    /// Fails with storage's error when what it holds can't be loaded, and
    /// when it holds an xy colour, a temperature outside 1000 to 30000 K or
    /// a Description past 16 KiB.
    pub fn with_output_persistence(
        mut self,
        persistence: Arc<dyn ColorOutputPersistence>,
    ) -> Result<Self, Error> {
        let Some(saved) = self.output.open(self.oid, persistence)? else {
            return Ok(self);
        };
        // A default may be 0, the restart rule's; Present_Value may not.
        let kelvin = |value, zero_too: bool| match value {
            None => Ok(None),
            Some(ColorOutputValue::Kelvin(kelvin))
                if command::KELVIN.contains(&kelvin) || (zero_too && kelvin == 0) =>
            {
                Ok(Some(kelvin))
            }
            Some(_) => Err(Error::Encoding(
                "saved Color Temperature output is not a temperature the object can hold".into(),
            )),
        };
        if let Some(kelvin) = kelvin(saved.present_value, false)? {
            self.present_value = self.clamped(kelvin);
            self.restored = true;
            self.to_restore = Some(kelvin);
        }
        if let Some(default) = kelvin(saved.default, true)? {
            self.default_color_temperature = match default {
                0 => 0,
                default => self.clamped(default),
            };
        }
        if let Some(text) = saved.description {
            self.description = text;
        }
        Ok(self)
    }

    /// Whether Present_Value came back from storage
    /// ([`with_output_persistence`](Self::with_output_persistence)).
    pub fn output_restored(&self) -> bool {
        self.restored
    }

    /// The output save counters, shared with the object, or `None` without
    /// storage. Take them before the object joins a database.
    pub fn output_save_counters(&self) -> Option<ColorOutputSaveCounters> {
        self.output.counters()
    }

    /// Block until the output saves queued so far have run. Dropping the
    /// object waits for them too.
    pub fn wait_for_output_saves(&self) {
        self.output.wait_for_saves();
    }

    /// Default_Color_Temperature in kelvin: the temperature the device's
    /// start drives the output to, or 0 to keep the one from before the
    /// restart.
    pub fn default_color_temperature(&self) -> u32 {
        self.default_color_temperature
    }

    /// Set Default_Color_Temperature, as a WriteProperty of it would: 0, or
    /// 1000 to 30000 K clamped to Min_Pres_Value and Max_Pres_Value (Clause
    /// 12.Y.8). Any other value is refused with VALUE_OUT_OF_RANGE.
    ///
    /// This configures the default; it is not saved. Once storage holds a
    /// default a write set ([`with_output_persistence`](Self::with_output_persistence)),
    /// that one wins, as on a Color object: the value is checked but not
    /// taken, and [`clear_written_default`](Self::clear_written_default)
    /// lets it apply again.
    pub fn set_default_color_temperature(&mut self, kelvin: u32) -> Result<(), Error> {
        let default = match kelvin {
            0 => 0,
            kelvin => self.written_kelvin(u64::from(kelvin))?,
        };
        if !self.output.default_saved() {
            self.default_color_temperature = default;
        }
        Ok(())
    }

    /// Whether storage holds a Default_Color_Temperature a write set, now or
    /// before a restart. Always false without storage.
    pub fn default_color_temperature_saved(&self) -> bool {
        self.output.default_saved()
    }

    /// Drop the Default_Color_Temperature a write saved (#1956), as a Color
    /// object's
    /// [`clear_written_default`](super::super::ColorObject::clear_written_default)
    /// does.
    /// Call it before the object joins the database, as the setters are,
    /// not under the database guard: it waits for its save.
    pub fn clear_written_default(&mut self) -> Result<(), Error> {
        self.output.clear_written_default()
    }

    /// Drop the Description a write saved (#2364), as a Color object's
    /// [`clear_written_description`](super::super::ColorObject::clear_written_description)
    /// does.
    /// Call it before the object joins the database, not under the database
    /// guard: it waits for its save.
    pub fn clear_written_description(&mut self) -> Result<(), Error> {
        self.output.clear_written_description()
    }

    /// A Default_Color_Temperature write. Clause 12.Y.8 clamps it as
    /// Present_Value's is, except 0, which the restart rule of Clause 12.Y.4
    /// gives a meaning of its own: bring back the temperature from before
    /// the restart. With storage it is saved first (#1956).
    pub(super) fn write_default_color_temperature(
        &mut self,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let (min, max) = (self.min_pres_value, self.max_pres_value);
        let property = PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE;
        let written = self
            .output
            .write_default(property, &value, || written_default(&value, min, max))?;
        if let ColorOutputValue::Kelvin(kelvin) = written {
            self.default_color_temperature = kelvin;
        }
        Ok(())
    }

    /// Stage the request's Default_Color_Temperature and Description writes
    /// on the output storage.
    pub(super) fn stage_default(&mut self, writes: &[PendingWrite]) -> StageStep {
        let (min, max) = (self.min_pres_value, self.max_pres_value);
        self.output.stage_kept(
            writes,
            PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE,
            |value| written_default(value, min, max),
        )
    }
}
