use super::CommandOrigin;
use crate::common;
use bacnet_encoding::{
    constructed::{decode_value_source, encode_value_source},
    primitives::encode_timestamp_choice,
};
use bacnet_types::{
    constructed::BACnetValueSource,
    enums::PropertyIdentifier as P,
    error::Error,
    primitives::{BACnetTimeStamp, PropertyValue},
};
use bytes::BytesMut;

#[derive(Debug, Clone)]
pub(crate) struct ValueSourceTracking {
    sources: [BACnetValueSource; 16],
    owners: [Option<CommandOrigin>; 16],
    sequence: u16,
}
impl Default for ValueSourceTracking {
    fn default() -> Self {
        Self {
            sources: std::array::from_fn(|_| BACnetValueSource::None),
            owners: std::array::from_fn(|_| None),
            sequence: 0,
        }
    }
}
/// The source a correction claims: one complete BACnetValueSource and
/// nothing after it, or INVALID_DATA_TYPE.
pub(super) fn decode_claim(value: PropertyValue) -> Result<BACnetValueSource, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    let (source, end) =
        decode_value_source(&bytes, 0).map_err(|_| common::invalid_data_type_error())?;
    if end != bytes.len() {
        return Err(common::invalid_data_type_error());
    }
    Ok(source)
}
fn active<T>(slots: &[Option<T>; 16]) -> Option<usize> {
    slots.iter().position(Option::is_some)
}
fn priority_index(priority: Option<u8>) -> Result<usize, Error> {
    let priority = priority.unwrap_or(16);
    if (1..=16).contains(&priority) {
        Ok(usize::from(priority - 1))
    } else {
        Err(common::value_out_of_range_error())
    }
}
impl ValueSourceTracking {
    fn current<T>(&self, slots: &[Option<T>; 16]) -> BACnetValueSource {
        active(slots).map_or(BACnetValueSource::None, |i| self.sources[i].clone())
    }
    pub(crate) fn command<T: Copy + PartialEq>(
        &mut self,
        slots: &mut [Option<T>; 16],
        fallback: T,
        value: Option<T>,
        priority: Option<u8>,
        origin: &CommandOrigin,
    ) -> Result<(), Error> {
        let index = priority_index(priority)?;
        origin.validate()?;
        let before = (
            common::recalculate_from_priority_array(slots, fallback),
            active(slots),
            self.current(slots),
        );
        slots[index] = value;
        // A relinquishment is a command: retain its writer at this inactive slot.
        self.sources[index] = origin.published_source();
        self.owners[index] = Some(origin.clone());
        let after = (
            common::recalculate_from_priority_array(slots, fallback),
            active(slots),
            self.current(slots),
        );
        if before != after {
            self.sequence = self.sequence.wrapping_add(1);
        }
        Ok(())
    }
    pub(crate) fn correct(
        &mut self,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &CommandOrigin,
    ) -> Result<(), Error> {
        let index = priority_index(priority)?;
        origin.validate()?;
        // Ownership comes before the value: a requester who may not correct
        // this slot is told so whatever it wrote, a NULL included (#1396).
        if !self.owners[index]
            .as_ref()
            .is_some_and(|owner| owner.permits_correction_by(origin))
        {
            return Err(common::write_access_denied_error());
        }
        self.sources[index] = decode_claim(value)?;
        // Never replace the original command owner or timestamp on correction.
        Ok(())
    }
    pub(crate) fn read<T>(
        &self,
        property: P,
        index: Option<u32>,
        slots: &[Option<T>; 16],
    ) -> Option<Result<PropertyValue, Error>> {
        if !matches!(
            property,
            P::VALUE_SOURCE | P::VALUE_SOURCE_ARRAY | P::LAST_COMMAND_TIME
        ) {
            return None;
        }
        Some(self.read_inner(property, index, slots))
    }
    fn read_inner<T>(
        &self,
        property: P,
        index: Option<u32>,
        slots: &[Option<T>; 16],
    ) -> Result<PropertyValue, Error> {
        if property != P::VALUE_SOURCE_ARRAY && index.is_some() {
            return Err(common::property_is_not_an_array_error());
        }
        let mut bytes = BytesMut::new();
        match property {
            P::VALUE_SOURCE => encode_value_source(&mut bytes, &self.current(slots))?,
            P::LAST_COMMAND_TIME => encode_timestamp_choice(
                &mut bytes,
                &BACnetTimeStamp::SequenceNumber(self.sequence),
            )?,
            P::VALUE_SOURCE_ARRAY => match index {
                Some(0) => return Ok(PropertyValue::Unsigned(16)),
                Some(i @ 1..=16) => {
                    encode_value_source(&mut bytes, &self.sources[(i - 1) as usize])?
                }
                None => {
                    for source in &self.sources {
                        encode_value_source(&mut bytes, source)?;
                    }
                }
                _ => return Err(common::invalid_array_index_error()),
            },
            _ => unreachable!(),
        }
        Ok(PropertyValue::ApplicationData(bytes.to_vec()))
    }
}

// Family-specific extraction validates the value before the shared state changes.
macro_rules! write_sourced_priority {
    ($self:expr, $value:expr, $priority:expr, $origin:expr, $extract:expr) => {{
        let prio = $priority.unwrap_or(16);
        if !(1..=16).contains(&prio) {
            return Err($crate::common::value_out_of_range_error());
        }
        let command = match $value {
            bacnet_types::primitives::PropertyValue::Null => None,
            value => Some(($extract)(value)?),
        };
        $self.value_source.command(
            &mut $self.priority_array,
            $self.relinquish_default,
            command,
            $priority,
            $origin,
        )?;
        $self.recalculate_present_value();
        Ok(())
    }};
}
pub(crate) use write_sourced_priority;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_source_sequence_wraps_on_effective_change_only() {
        let mut tracking = ValueSourceTracking {
            sequence: u16::MAX,
            ..Default::default()
        };
        let mut slots = [None; 16];
        let origin = CommandOrigin::Local {
            owner_device: bacnet_types::primitives::ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::DEVICE,
                1,
            )
            .unwrap(),
            initiating_object: None,
        };
        tracking
            .command(&mut slots, 0, Some(1), Some(8), &origin)
            .unwrap();
        assert_eq!(tracking.sequence, 0);
        tracking
            .command(&mut slots, 0, Some(1), Some(8), &origin)
            .unwrap();
        assert_eq!(tracking.sequence, 0);
        tracking
            .command(&mut slots, 0, Some(2), Some(8), &origin)
            .unwrap();
        assert_eq!(tracking.sequence, 1);
    }
}
