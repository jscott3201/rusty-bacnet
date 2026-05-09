use super::*;

impl NotificationParameters {
    /// Encode notification parameters into the buffer.
    ///
    /// Each variant is wrapped in its own opening/closing tag pair
    /// matching the variant's context tag number.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        match self {
            Self::ChangeOfBitstring {
                referenced_bitstring,
                status_flags,
            } => {
                tags::encode_opening_tag(buf, 0);
                primitives::encode_ctx_bit_string(
                    buf,
                    0,
                    referenced_bitstring.0,
                    &referenced_bitstring.1,
                );
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                tags::encode_closing_tag(buf, 0);
            }
            Self::ChangeOfState {
                new_state,
                status_flags,
            } => {
                tags::encode_opening_tag(buf, 1);
                // [0] new-state: BACnetPropertyStates — wrapped in opening/closing [0]
                tags::encode_opening_tag(buf, 0);
                encode_property_states(buf, new_state);
                tags::encode_closing_tag(buf, 0);
                // [1] status-flags
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                tags::encode_closing_tag(buf, 1);
            }
            Self::ChangeOfValue {
                new_value,
                status_flags,
            } => {
                tags::encode_opening_tag(buf, 2);
                // [0] new-value: CHOICE — wrapped in opening/closing [0]
                tags::encode_opening_tag(buf, 0);
                match new_value {
                    ChangeOfValueChoice::ChangedBits { unused_bits, data } => {
                        primitives::encode_ctx_bit_string(buf, 0, *unused_bits, data);
                    }
                    ChangeOfValueChoice::ChangedValue(v) => {
                        primitives::encode_ctx_real(buf, 1, *v);
                    }
                }
                tags::encode_closing_tag(buf, 0);
                // [1] status-flags
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                tags::encode_closing_tag(buf, 2);
            }
            Self::CommandFailure {
                command_value,
                status_flags,
                feedback_value,
            } => {
                tags::encode_opening_tag(buf, 3);
                // [0] command-value — abstract syntax, encoded as raw octet string
                tags::encode_opening_tag(buf, 0);
                buf.extend_from_slice(command_value);
                tags::encode_closing_tag(buf, 0);
                // [1] status-flags
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                // [2] feedback-value — abstract syntax, encoded as raw
                tags::encode_opening_tag(buf, 2);
                buf.extend_from_slice(feedback_value);
                tags::encode_closing_tag(buf, 2);
                tags::encode_closing_tag(buf, 3);
            }
            Self::FloatingLimit {
                reference_value,
                status_flags,
                setpoint_value,
                error_limit,
            } => {
                tags::encode_opening_tag(buf, 4);
                primitives::encode_ctx_real(buf, 0, *reference_value);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_real(buf, 2, *setpoint_value);
                primitives::encode_ctx_real(buf, 3, *error_limit);
                tags::encode_closing_tag(buf, 4);
            }
            Self::OutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                tags::encode_opening_tag(buf, 5);
                primitives::encode_ctx_real(buf, 0, *exceeding_value);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_real(buf, 2, *deadband);
                primitives::encode_ctx_real(buf, 3, *exceeded_limit);
                tags::encode_closing_tag(buf, 5);
            }
            Self::ChangeOfLifeSafety {
                new_state,
                new_mode,
                status_flags,
                operation_expected,
            } => {
                tags::encode_opening_tag(buf, 8);
                primitives::encode_ctx_enumerated(buf, 0, *new_state);
                primitives::encode_ctx_enumerated(buf, 1, *new_mode);
                primitives::encode_ctx_bit_string(buf, 2, 4, &[*status_flags << 4]);
                primitives::encode_ctx_enumerated(buf, 3, *operation_expected);
                tags::encode_closing_tag(buf, 8);
            }
            Self::Extended {
                vendor_id,
                extended_event_type,
                parameters,
            } => {
                tags::encode_opening_tag(buf, 9);
                primitives::encode_ctx_unsigned(buf, 0, *vendor_id as u64);
                primitives::encode_ctx_unsigned(buf, 1, *extended_event_type as u64);
                // [2] parameters — raw
                tags::encode_opening_tag(buf, 2);
                buf.extend_from_slice(parameters);
                tags::encode_closing_tag(buf, 2);
                tags::encode_closing_tag(buf, 9);
            }
            Self::BufferReady {
                buffer_property,
                previous_notification,
                current_notification,
            } => {
                tags::encode_opening_tag(buf, 10);
                // [0] buffer-property: BACnetDeviceObjectPropertyReference
                tags::encode_opening_tag(buf, 0);
                primitives::encode_ctx_object_id(buf, 0, &buffer_property.object_identifier);
                primitives::encode_ctx_unsigned(buf, 1, buffer_property.property_identifier as u64);
                if let Some(idx) = buffer_property.property_array_index {
                    primitives::encode_ctx_unsigned(buf, 2, idx as u64);
                }
                if let Some(ref dev) = buffer_property.device_identifier {
                    primitives::encode_ctx_object_id(buf, 3, dev);
                }
                tags::encode_closing_tag(buf, 0);
                // [1] previous-notification
                primitives::encode_ctx_unsigned(buf, 1, *previous_notification as u64);
                // [2] current-notification
                primitives::encode_ctx_unsigned(buf, 2, *current_notification as u64);
                tags::encode_closing_tag(buf, 10);
            }
            Self::UnsignedRange {
                exceeding_value,
                status_flags,
                exceeded_limit,
            } => {
                tags::encode_opening_tag(buf, 11);
                primitives::encode_ctx_unsigned(buf, 0, *exceeding_value);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_unsigned(buf, 2, *exceeded_limit);
                tags::encode_closing_tag(buf, 11);
            }
            Self::AccessEvent {
                access_event,
                status_flags,
                access_event_tag,
                access_event_time,
                access_credential,
                authentication_factor,
            } => {
                tags::encode_opening_tag(buf, 13);
                primitives::encode_ctx_enumerated(buf, 0, *access_event);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_unsigned(buf, 2, *access_event_tag as u64);
                // [3] access-event-time: BACnetTimeStamp (DateTime)
                primitives::encode_timestamp(
                    buf,
                    3,
                    &BACnetTimeStamp::DateTime {
                        date: access_event_time.0,
                        time: access_event_time.1,
                    },
                );
                // [4] access-credential: BACnetDeviceObjectPropertyReference
                tags::encode_opening_tag(buf, 4);
                primitives::encode_ctx_object_id(buf, 0, &access_credential.object_identifier);
                primitives::encode_ctx_unsigned(
                    buf,
                    1,
                    access_credential.property_identifier as u64,
                );
                if let Some(idx) = access_credential.property_array_index {
                    primitives::encode_ctx_unsigned(buf, 2, idx as u64);
                }
                if let Some(ref dev) = access_credential.device_identifier {
                    primitives::encode_ctx_object_id(buf, 3, dev);
                }
                tags::encode_closing_tag(buf, 4);
                // [5] authentication-factor — raw
                tags::encode_opening_tag(buf, 5);
                buf.extend_from_slice(authentication_factor);
                tags::encode_closing_tag(buf, 5);
                tags::encode_closing_tag(buf, 13);
            }
            Self::DoubleOutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                tags::encode_opening_tag(buf, 14);
                primitives::encode_ctx_double(buf, 0, *exceeding_value);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_double(buf, 2, *deadband);
                primitives::encode_ctx_double(buf, 3, *exceeded_limit);
                tags::encode_closing_tag(buf, 14);
            }
            Self::SignedOutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                tags::encode_opening_tag(buf, 15);
                primitives::encode_ctx_signed(buf, 0, *exceeding_value);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_unsigned(buf, 2, *deadband);
                primitives::encode_ctx_signed(buf, 3, *exceeded_limit);
                tags::encode_closing_tag(buf, 15);
            }
            Self::UnsignedOutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                tags::encode_opening_tag(buf, 16);
                primitives::encode_ctx_unsigned(buf, 0, *exceeding_value);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_unsigned(buf, 2, *deadband);
                primitives::encode_ctx_unsigned(buf, 3, *exceeded_limit);
                tags::encode_closing_tag(buf, 16);
            }
            Self::ChangeOfCharacterstring {
                changed_value,
                status_flags,
                alarm_value,
            } => {
                tags::encode_opening_tag(buf, 17);
                primitives::encode_ctx_character_string(buf, 0, changed_value)?;
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                primitives::encode_ctx_character_string(buf, 2, alarm_value)?;
                tags::encode_closing_tag(buf, 17);
            }
            Self::ChangeOfStatusFlags {
                present_value,
                referenced_flags,
            } => {
                tags::encode_opening_tag(buf, 18);
                // [0] present-value — abstract syntax, raw
                tags::encode_opening_tag(buf, 0);
                buf.extend_from_slice(present_value);
                tags::encode_closing_tag(buf, 0);
                // [1] referenced-flags
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*referenced_flags << 4]);
                tags::encode_closing_tag(buf, 18);
            }
            Self::ChangeOfReliability {
                reliability,
                status_flags,
                property_values,
            } => {
                tags::encode_opening_tag(buf, 19);
                primitives::encode_ctx_enumerated(buf, 0, *reliability);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                // [2] property-values — abstract syntax, raw
                tags::encode_opening_tag(buf, 2);
                buf.extend_from_slice(property_values);
                tags::encode_closing_tag(buf, 2);
                tags::encode_closing_tag(buf, 19);
            }
            Self::NoneParams => {
                tags::encode_opening_tag(buf, 20);
                tags::encode_closing_tag(buf, 20);
            }
            Self::ChangeOfDiscreteValue {
                new_value,
                status_flags,
            } => {
                tags::encode_opening_tag(buf, 21);
                // [0] new-value — abstract syntax, raw
                tags::encode_opening_tag(buf, 0);
                buf.extend_from_slice(new_value);
                tags::encode_closing_tag(buf, 0);
                // [1] status-flags
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                tags::encode_closing_tag(buf, 21);
            }
            Self::ChangeOfTimer {
                new_state,
                status_flags,
                update_time,
                last_state_change,
                initial_timeout,
                expiration_time,
            } => {
                tags::encode_opening_tag(buf, 22);
                primitives::encode_ctx_enumerated(buf, 0, *new_state);
                primitives::encode_ctx_bit_string(buf, 1, 4, &[*status_flags << 4]);
                // [2] update-time: BACnetDateTime
                tags::encode_opening_tag(buf, 2);
                primitives::encode_app_date(buf, &update_time.0);
                primitives::encode_app_time(buf, &update_time.1);
                tags::encode_closing_tag(buf, 2);
                // [3] last-state-change (optional enumerated — always encode)
                primitives::encode_ctx_enumerated(buf, 3, *last_state_change);
                // [4] initial-timeout (optional unsigned — always encode)
                primitives::encode_ctx_unsigned(buf, 4, *initial_timeout as u64);
                // [5] expiration-time: BACnetDateTime
                tags::encode_opening_tag(buf, 5);
                primitives::encode_app_date(buf, &expiration_time.0);
                primitives::encode_app_time(buf, &expiration_time.1);
                tags::encode_closing_tag(buf, 5);
                tags::encode_closing_tag(buf, 22);
            }
        }
        Ok(())
    }
}
