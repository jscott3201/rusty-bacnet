//! The LEGACY flat [`PropertyValue::List`] form of [`BACnetEventParameter`]
//! (#154): the algorithm tag first, then the variant's fields in
//! declaration order.
//!
//! Split out of `mod.rs` to keep every file under the 700-LOC cap.

use super::*;

/// An Unsigned item.
fn unsigned(value: impl Into<u64>) -> PropertyValue {
    PropertyValue::Unsigned(value.into())
}

/// A list of enumerated items.
fn enumerations(values: impl Iterator<Item = u32>) -> PropertyValue {
    PropertyValue::List(values.map(PropertyValue::Enumerated).collect())
}

impl BACnetEventParameter {
    /// Encode this parameter set as a flat [`PropertyValue::List`].
    ///
    /// The first element is the algorithm tag as [`PropertyValue::Unsigned`],
    /// followed by the variant's fields in declaration order. This is the
    /// LEGACY pre-framing layout (see #154); the wire form is the full ASN.1
    /// framing from `bacnet_encoding::constructed::encode_event_parameter`.
    /// See [`Self::decode`] for the inverse.
    pub fn encode(&self) -> PropertyValue {
        let mut items = vec![PropertyValue::Unsigned(self.tag() as u64)];
        match self {
            Self::ChangeOfBitstring {
                time_delay,
                bitmask,
                list_of_values,
            } => {
                items.push(unsigned(*time_delay));
                items.push(bitstring_pv(bitmask.clone()));
                items.push(PropertyValue::List(
                    list_of_values
                        .iter()
                        .map(|bs| bitstring_pv(bs.clone()))
                        .collect(),
                ));
            }
            Self::ChangeOfState {
                time_delay,
                list_of_values,
            } => {
                items.push(unsigned(*time_delay));
                items.push(PropertyValue::List(
                    list_of_values.iter().map(property_state_pv).collect(),
                ));
            }
            Self::ChangeOfValue {
                time_delay,
                criteria,
            } => {
                items.push(unsigned(*time_delay));
                items.push(match criteria {
                    ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                        PropertyValue::BitString {
                            unused_bits: *unused_bits,
                            data: data.clone(),
                        }
                    }
                    ChangeOfValueCriteria::ReferencedPropertyIncrement(inc) => {
                        PropertyValue::Real(*inc)
                    }
                });
            }
            Self::CommandFailure {
                time_delay,
                feedback_property_reference,
            } => {
                items.push(unsigned(*time_delay));
                items.push(device_object_property_reference_pv(
                    feedback_property_reference,
                ));
            }
            Self::FloatingLimit {
                time_delay,
                setpoint_reference,
                low_diff_limit,
                high_diff_limit,
                deadband,
            } => items.extend([
                unsigned(*time_delay),
                device_object_property_reference_pv(setpoint_reference),
                PropertyValue::Real(*low_diff_limit),
                PropertyValue::Real(*high_diff_limit),
                PropertyValue::Real(*deadband),
            ]),
            Self::OutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            } => items.extend([
                unsigned(*time_delay),
                PropertyValue::Real(*low_limit),
                PropertyValue::Real(*high_limit),
                PropertyValue::Real(*deadband),
            ]),
            Self::ChangeOfLifeSafety {
                time_delay,
                list_of_life_safety_alarm_values,
                list_of_alarm_values,
                mode_property_reference,
            } => items.extend([
                unsigned(*time_delay),
                enumerations(list_of_life_safety_alarm_values.iter().map(|s| s.to_raw())),
                enumerations(list_of_alarm_values.iter().map(|s| s.to_raw())),
                device_object_property_reference_pv(mode_property_reference),
            ]),
            Self::Extended {
                vendor_id,
                extended_event_type,
                parameters,
            } => items.extend([
                unsigned(*vendor_id),
                unsigned(*extended_event_type),
                PropertyValue::OctetString(parameters.clone()),
            ]),
            Self::BufferReady {
                notification_threshold,
                previous_notification_count,
            } => items.extend([
                unsigned(*notification_threshold),
                unsigned(*previous_notification_count),
            ]),
            Self::UnsignedRange {
                time_delay,
                low_limit,
                high_limit,
            } => items.extend([
                unsigned(*time_delay),
                unsigned(*low_limit),
                unsigned(*high_limit),
            ]),
            Self::AccessEvent {
                list_of_access_events,
                access_event_time_reference,
            } => items.extend([
                enumerations(list_of_access_events.iter().map(|e| e.to_raw())),
                device_object_property_reference_pv(access_event_time_reference),
            ]),
            Self::DoubleOutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            } => items.extend([
                unsigned(*time_delay),
                PropertyValue::Double(*low_limit),
                PropertyValue::Double(*high_limit),
                PropertyValue::Double(*deadband),
            ]),
            Self::SignedOutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            } => items.extend([
                unsigned(*time_delay),
                PropertyValue::Signed(*low_limit),
                PropertyValue::Signed(*high_limit),
                unsigned(*deadband),
            ]),
            Self::UnsignedOutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            } => items.extend([
                unsigned(*time_delay),
                unsigned(*low_limit),
                unsigned(*high_limit),
                unsigned(*deadband),
            ]),
            Self::ChangeOfCharacterstring {
                time_delay,
                list_of_alarm_values,
            } => items.extend([
                unsigned(*time_delay),
                PropertyValue::List(
                    list_of_alarm_values
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                ),
            ]),
            Self::ChangeOfStatusFlags {
                time_delay,
                selected_flags,
            } => items.extend([
                unsigned(*time_delay),
                PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![selected_flags.bits() << 4],
                },
            ]),
            Self::None => {}
            Self::ChangeOfDiscreteValue { time_delay } => items.push(unsigned(*time_delay)),
            Self::ChangeOfTimer {
                time_delay,
                alarm_values,
                update_time_reference,
            } => items.extend([
                unsigned(*time_delay),
                enumerations(alarm_values.iter().map(|s| s.to_raw())),
                device_object_property_reference_pv(update_time_reference),
            ]),
            Self::Opaque { tag: _, data } => items.push(PropertyValue::OctetString(data.clone())),
        }
        PropertyValue::List(items)
    }

    /// Decode a [`PropertyValue`] previously produced by [`Self::encode`].
    ///
    /// Returns `Ok` with the reconstructed parameter set, or a decoding
    /// [`Error`] when the value is not a `List`, is empty, or has an
    /// unrecognized/inconsistent shape. `Opaque` values round-trip unchanged.
    pub fn decode(value: &PropertyValue) -> Result<Self, Error> {
        use event_parameter_tag as T;
        let PropertyValue::List(items) = value else {
            return Err(Error::decoding(0, "Event_Parameters is not a List"));
        };
        let Some((tag_pv, rest)) = items.split_first() else {
            return Err(Error::decoding(0, "Event_Parameters list is empty"));
        };
        let PropertyValue::Unsigned(tag) = tag_pv else {
            return Err(Error::decoding(0, "Event_Parameters tag is not Unsigned"));
        };
        let tag = u8::try_from(*tag)
            .map_err(|_| Error::decoding(0, "Event_Parameters tag exceeds 255"))?;
        let mut idx = 0;
        let i = &mut idx;
        let parameters = match tag {
            T::OUT_OF_RANGE => Self::OutOfRange {
                time_delay: take_u32(rest, i)?,
                low_limit: take_real(rest, i)?,
                high_limit: take_real(rest, i)?,
                deadband: take_real(rest, i)?,
            },
            T::FLOATING_LIMIT => Self::FloatingLimit {
                time_delay: take_u32(rest, i)?,
                setpoint_reference: take_dopr(rest, i)?,
                low_diff_limit: take_real(rest, i)?,
                high_diff_limit: take_real(rest, i)?,
                deadband: take_real(rest, i)?,
            },
            T::CHANGE_OF_STATE => Self::ChangeOfState {
                time_delay: take_u32(rest, i)?,
                list_of_values: take_property_state_list(rest, i)?,
            },
            T::CHANGE_OF_BITSTRING => Self::ChangeOfBitstring {
                time_delay: take_u32(rest, i)?,
                bitmask: take_bitstring(rest, i)?,
                list_of_values: take_bitstring_list(rest, i)?,
            },
            T::CHANGE_OF_VALUE => {
                let time_delay = take_u32(rest, i)?;
                let criteria = match take_pv(rest, i) {
                    Some(PropertyValue::BitString { unused_bits, data }) => {
                        ChangeOfValueCriteria::Bitmask {
                            unused_bits: *unused_bits,
                            data: data.clone(),
                        }
                    }
                    Some(PropertyValue::Real(inc)) => {
                        ChangeOfValueCriteria::ReferencedPropertyIncrement(*inc)
                    }
                    _ => return Err(Error::decoding(*i, "invalid cov-criteria")),
                };
                Self::ChangeOfValue {
                    time_delay,
                    criteria,
                }
            }
            T::COMMAND_FAILURE => Self::CommandFailure {
                time_delay: take_u32(rest, i)?,
                feedback_property_reference: take_dopr(rest, i)?,
            },
            T::CHANGE_OF_LIFE_SAFETY => Self::ChangeOfLifeSafety {
                time_delay: take_u32(rest, i)?,
                list_of_life_safety_alarm_values: take_enum_list(rest, i)?
                    .map(LifeSafetyState::from_raw)
                    .collect(),
                list_of_alarm_values: take_enum_list(rest, i)?
                    .map(LifeSafetyState::from_raw)
                    .collect(),
                mode_property_reference: take_dopr(rest, i)?,
            },
            T::EXTENDED => {
                let vendor_id = take_u16(rest, i)?;
                let extended_event_type = take_u32(rest, i)?;
                let parameters = match take_pv(rest, i) {
                    Some(PropertyValue::OctetString(bytes)) => bytes.clone(),
                    _ => return Err(Error::decoding(*i, "extended parameters not octets")),
                };
                Self::Extended {
                    vendor_id,
                    extended_event_type,
                    parameters,
                }
            }
            T::BUFFER_READY => Self::BufferReady {
                notification_threshold: take_u32(rest, i)?,
                previous_notification_count: take_u32(rest, i)?,
            },
            T::UNSIGNED_RANGE => Self::UnsignedRange {
                time_delay: take_u32(rest, i)?,
                low_limit: take_u64(rest, i)?,
                high_limit: take_u64(rest, i)?,
            },
            T::ACCESS_EVENT => Self::AccessEvent {
                list_of_access_events: take_enum_list(rest, i)?
                    .map(AccessEvent::from_raw)
                    .collect(),
                access_event_time_reference: take_dopr(rest, i)?,
            },
            T::DOUBLE_OUT_OF_RANGE => Self::DoubleOutOfRange {
                time_delay: take_u32(rest, i)?,
                low_limit: take_double(rest, i)?,
                high_limit: take_double(rest, i)?,
                deadband: take_double(rest, i)?,
            },
            T::SIGNED_OUT_OF_RANGE => Self::SignedOutOfRange {
                time_delay: take_u32(rest, i)?,
                low_limit: take_signed(rest, i)?,
                high_limit: take_signed(rest, i)?,
                deadband: take_u64(rest, i)?,
            },
            T::UNSIGNED_OUT_OF_RANGE => Self::UnsignedOutOfRange {
                time_delay: take_u32(rest, i)?,
                low_limit: take_u64(rest, i)?,
                high_limit: take_u64(rest, i)?,
                deadband: take_u64(rest, i)?,
            },
            T::CHANGE_OF_CHARACTERSTRING => Self::ChangeOfCharacterstring {
                time_delay: take_u32(rest, i)?,
                list_of_alarm_values: take_string_list(rest, i)?,
            },
            T::CHANGE_OF_STATUS_FLAGS => Self::ChangeOfStatusFlags {
                time_delay: take_u32(rest, i)?,
                selected_flags: take_status_flags(rest, i)?,
            },
            // Before `none` had a variant it was kept as an empty opaque
            // value, which wrote one empty octet string after the tag.
            T::NONE => {
                if let Some(PropertyValue::OctetString(bytes)) = rest.first()
                    && bytes.is_empty()
                {
                    *i += 1;
                }
                Self::None
            }
            T::CHANGE_OF_DISCRETE_VALUE => Self::ChangeOfDiscreteValue {
                time_delay: take_u32(rest, i)?,
            },
            T::CHANGE_OF_TIMER => Self::ChangeOfTimer {
                time_delay: take_u32(rest, i)?,
                alarm_values: take_enum_list(rest, i)?.map(TimerState::from_raw).collect(),
                update_time_reference: take_dopr(rest, i)?,
            },
            tag => {
                let data = match take_pv(rest, i) {
                    Some(PropertyValue::OctetString(bytes)) => bytes.clone(),
                    Some(_) => return Err(Error::decoding(*i, "opaque payload not octets")),
                    None => Vec::new(),
                };
                Self::Opaque { tag, data }
            }
        };
        if idx != rest.len() {
            return Err(Error::decoding(idx, "Event_Parameters has trailing values"));
        }
        Ok(parameters)
    }
}
