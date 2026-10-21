//! Table 13-1a-2 Value_Source capture, shared by Single and Multiple.
use super::{flags::PreparedFlags, prepare::PreparedCovValue, CovObservation, CovSample};
use bacnet_encoding::{constructed::decode_value_source, primitives::decode_timestamp_choice};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_types::{
    enums::{ObjectType as O, PropertyIdentifier as P},
    error::Error,
    primitives::PropertyValue as V,
};

/// The report's fields in Table 13-1a-2 order: Present_Value, Status_Flags
/// when the object has it, Value_Source, then Last_Command_Time and
/// Current_Command_Priority for a commandable object.
fn fields(commandable: bool, flags: bool) -> &'static [P] {
    match (commandable, flags) {
        (true, _) => &[
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::VALUE_SOURCE,
            P::LAST_COMMAND_TIME,
            P::CURRENT_COMMAND_PRIORITY,
        ],
        (false, true) => &[P::PRESENT_VALUE, P::STATUS_FLAGS, P::VALUE_SOURCE],
        // Color and Color Temperature have no Status_Flags to convey (#1582).
        (false, false) => &[P::PRESENT_VALUE, P::VALUE_SOURCE],
    }
}

/// Whether a subscription to `property` of `object` takes the specialized
/// Table 13-1a-2 report. Commandable outputs and values, and the two
/// lighting outputs (#1866), use the commandable row. The single-source
/// objects that track Value_Source (#1552: the noncommandable value modes,
/// Color and Color Temperature) use the noncommandable row (#1582).
pub(crate) fn applies(object: &dyn BACnetObject, property: P) -> bool {
    if property != P::VALUE_SOURCE {
        return false;
    }
    let properties = object.property_list();
    let commandable = properties.contains(&P::PRIORITY_ARRAY);
    match object.object_identifier().object_type() {
        O::ANALOG_OUTPUT
        | O::BINARY_OUTPUT
        | O::MULTI_STATE_OUTPUT
        | O::LIGHTING_OUTPUT
        | O::BINARY_LIGHTING_OUTPUT => commandable,
        O::ANALOG_VALUE | O::BINARY_VALUE | O::MULTI_STATE_VALUE => {
            commandable || properties.contains(&P::VALUE_SOURCE)
        }
        O::COLOR | O::COLOR_TEMPERATURE => properties.contains(&P::VALUE_SOURCE),
        _ => false,
    }
}

/// Owns the complete validated report and baseline from one object borrow. Other
/// Multiple selectors reuse these captured values, never read a second version.
pub(crate) struct PreparedValueSource {
    properties: &'static [P],
    fields: Vec<PreparedCovValue>,
    /// Index of Current_Command_Priority in `fields`, for a commandable object.
    priority: Option<usize>,
    pub observation: CovObservation,
}
impl PreparedValueSource {
    pub fn read(object: &dyn BACnetObject, flags: &PreparedFlags) -> Result<Self, Error> {
        let commandable = object.property_list().contains(&P::PRIORITY_ARRAY);
        let properties = fields(commandable, flags.value.is_some());
        let mut fields = Vec::with_capacity(properties.len());
        for &property in properties {
            let value = if property == P::STATUS_FLAGS {
                flags.value.clone().ok_or_else(invalid)?
            } else {
                object.read_property(property, None)?
            };
            // Bound before inspecting constructed payloads or retaining copies.
            CovSample::new(&value)?;
            validate(object.object_identifier().object_type(), property, &value)?;
            fields.push(super::prepare::prepare_value(
                object, property, None, None, &value,
            )?);
        }
        // Only an analog or Lighting Output PV uses the object's increment
        // (Table 13-1). A selected Value_Source increment must never replace
        // the object's Table 13-1 criterion. Color and Color Temperature are
        // not in Table 13-1 and have no COV_Increment, so their Present_Value
        // reports on any change.
        if !matches!(
            object.object_identifier().object_type(),
            O::ANALOG_OUTPUT | O::ANALOG_VALUE | O::LIGHTING_OUTPUT
        ) {
            fields[0].increment = None;
        }
        let source = position(properties, P::VALUE_SOURCE);
        let priority = commandable.then(|| position(properties, P::CURRENT_COMMAND_PRIORITY));
        let observation = flags
            .observation(fields[source].sample.clone())
            .with_source_companions(
                fields[0].sample.clone(),
                priority.map(|i| fields[i].sample.clone()),
            );
        Ok(Self {
            properties,
            fields,
            priority,
            observation,
        })
    }
    pub fn value(&self, property: P) -> Option<&V> {
        self.properties
            .iter()
            .position(|p| *p == property)
            .map(|i| self.fields[i].sample.value())
    }
    pub fn reports(&self, previous: Option<&CovObservation>) -> bool {
        let Some(previous) = previous else {
            return true;
        };
        let Some((pv, priority)) = previous.source_companions() else {
            return true;
        };
        self.fields[0].reports(Some(pv))
            || self.observation.flags_changed(Some(previous))
            || self.observation.sample() != previous.sample()
            || self.priority.map(|i| &self.fields[i].sample) != priority.as_ref()
    }
    pub fn values(&self) -> Vec<BACnetPropertyValue> {
        self.properties
            .iter()
            .copied()
            .zip(&self.fields)
            .map(|(property_identifier, field)| BACnetPropertyValue {
                property_identifier,
                property_array_index: None,
                value: field.encoded.clone(),
                priority: None,
            })
            .collect()
    }
}
fn position(properties: &[P], property: P) -> usize {
    properties
        .iter()
        .position(|p| *p == property)
        .expect("every Table 13-1a-2 field list carries this property")
}
fn invalid() -> Error {
    Error::Encoding("Invalid Value_Source COV companion".into())
}
fn validate(object: O, property: P, value: &V) -> Result<(), Error> {
    let valid = match property {
        P::PRESENT_VALUE => match object {
            O::ANALOG_OUTPUT | O::ANALOG_VALUE => matches!(value, V::Real(v) if v.is_finite()),
            // A level on the normalized 0.0 to 100.0 scale (Clause 12.54.4).
            O::LIGHTING_OUTPUT => matches!(value, V::Real(v) if (0.0..=100.0).contains(v)),
            // A Binary Lighting Output holds only OFF or ON; the warn and
            // stop values are commands, never a Present_Value (Clause 12.55).
            O::BINARY_OUTPUT | O::BINARY_VALUE | O::BINARY_LIGHTING_OUTPUT => {
                matches!(value, V::Enumerated(0..=1))
            }
            // A BACnetxyColor: the two finite REAL coordinates.
            O::COLOR => matches!(value, V::List(xy) if xy.len() == 2
                && xy.iter().all(|c| matches!(c, V::Real(v) if v.is_finite()))),
            // Kelvin, within the object's 1000 to 30000 range.
            O::COLOR_TEMPERATURE => matches!(value, V::Unsigned(1000..=30_000)),
            _ => matches!(value, V::Unsigned(1..=0xffff_ffff)),
        },
        P::VALUE_SOURCE => {
            matches!(value, V::ApplicationData(bytes) if decode_value_source(bytes, 0).is_ok_and(|(_, end)| end == bytes.len()))
        }
        P::LAST_COMMAND_TIME => {
            matches!(value, V::ApplicationData(bytes) if decode_timestamp_choice(bytes, 0).is_ok_and(|(_, end)| end == bytes.len()))
        }
        P::CURRENT_COMMAND_PRIORITY => matches!(value, V::Null | V::Unsigned(1..=16)),
        P::STATUS_FLAGS => super::observation::validate_flags(value).is_ok(),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid())
    }
}
