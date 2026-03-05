//! Alarm and event services per ASHRAE 135-2020 Clauses 13.2–13.9.
//!
//! - AcknowledgeAlarm (Clause 13.3)
//! - ConfirmedEventNotification / UnconfirmedEventNotification (Clause 13.5/13.6)
//! - GetEventInformation (Clause 13.9)

use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetPropertyStates};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// AcknowledgeAlarm (Clause 13.3)
// ---------------------------------------------------------------------------

/// AcknowledgeAlarm-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct AcknowledgeAlarmRequest {
    pub acknowledging_process_identifier: u32,
    pub event_object_identifier: ObjectIdentifier,
    pub event_state_acknowledged: u32,
    pub timestamp: BACnetTimeStamp,
    pub acknowledgment_source: String,
}

impl AcknowledgeAlarmRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] acknowledgingProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.acknowledging_process_identifier as u64);
        // [1] eventObjectIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.event_object_identifier);
        // [2] eventStateAcknowledged
        primitives::encode_ctx_enumerated(buf, 2, self.event_state_acknowledged);
        // [3] timestamp
        primitives::encode_timestamp(buf, 3, &self.timestamp);
        // [4] acknowledgmentSource
        primitives::encode_ctx_character_string(buf, 4, &self.acknowledgment_source)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0]
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "AcknowledgeAlarm truncated at process-id",
            ));
        }
        let acknowledging_process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1]
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "AcknowledgeAlarm truncated at object-id",
            ));
        }
        let event_object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [2]
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "AcknowledgeAlarm truncated at event-state",
            ));
        }
        let event_state_acknowledged = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [3] timestamp
        let (timestamp, new_offset) = primitives::decode_timestamp(data, offset, 3)?;
        offset = new_offset;

        // [4] acknowledgmentSource (required per Clause 13.3)
        let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 4)?;
        let acknowledgment_source = match opt_data {
            Some(content) => primitives::decode_character_string(content)?,
            None => {
                return Err(Error::decoding(
                    offset,
                    "AcknowledgeAlarm missing required acknowledgment-source [4]",
                ))
            }
        };

        Ok(Self {
            acknowledging_process_identifier,
            event_object_identifier,
            event_state_acknowledged,
            timestamp,
            acknowledgment_source,
        })
    }
}

// ---------------------------------------------------------------------------
// EventNotification (Clause 13.5 / 13.6)
// ---------------------------------------------------------------------------

/// ConfirmedEventNotification / UnconfirmedEventNotification request parameters.
///
/// Encodes all required fields per Clause 13.5/13.6. Event values (tag 12)
/// are still omitted (simplified).
#[derive(Debug, Clone)]
pub struct EventNotificationRequest {
    /// Process identifier of the notification recipient.
    pub process_identifier: u32,
    /// Device that generated the event.
    pub initiating_device_identifier: ObjectIdentifier,
    /// Object that triggered the event.
    pub event_object_identifier: ObjectIdentifier,
    /// Timestamp of the event transition.
    pub timestamp: BACnetTimeStamp,
    /// Notification class for routing.
    pub notification_class: u32,
    /// Priority (0-255).
    pub priority: u8,
    /// Event type (e.g., OUT_OF_RANGE = 5).
    pub event_type: u32,
    /// Notify type: ALARM(0), EVENT(1), ACK_NOTIFICATION(2).
    pub notify_type: u32,
    /// Whether the recipient must acknowledge.
    pub ack_required: bool,
    /// Event state before this transition.
    pub from_state: u32,
    /// Event state after this transition.
    pub to_state: u32,
    /// Optional event values (tag [12]).
    pub event_values: Option<NotificationParameters>,
}

impl EventNotificationRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] processIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.process_identifier as u64);
        // [1] initiatingDeviceIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.initiating_device_identifier);
        // [2] eventObjectIdentifier
        primitives::encode_ctx_object_id(buf, 2, &self.event_object_identifier);
        // [3] timeStamp
        primitives::encode_timestamp(buf, 3, &self.timestamp);
        // [4] notificationClass
        primitives::encode_ctx_unsigned(buf, 4, self.notification_class as u64);
        // [5] priority
        primitives::encode_ctx_unsigned(buf, 5, self.priority as u64);
        // [6] eventType
        primitives::encode_ctx_enumerated(buf, 6, self.event_type);
        // [7] messageText — omitted
        // [8] notifyType
        primitives::encode_ctx_enumerated(buf, 8, self.notify_type);
        // [9] ackRequired (only for ALARM/EVENT)
        if self.notify_type != 2 {
            primitives::encode_ctx_boolean(buf, 9, self.ack_required);
        }
        // [10] fromState
        primitives::encode_ctx_enumerated(buf, 10, self.from_state);
        // [11] toState
        primitives::encode_ctx_enumerated(buf, 11, self.to_state);
        // [12] eventValues — optional
        if let Some(ref params) = self.event_values {
            tags::encode_opening_tag(buf, 12);
            params.encode(buf)?;
            tags::encode_closing_tag(buf, 12);
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // Helper: validate bounds after computing end from tag length
        macro_rules! check_bounds {
            ($pos:expr, $end:expr, $field:expr) => {
                if $end > data.len() {
                    return Err(Error::decoding(
                        $pos,
                        concat!("EventNotification truncated at ", $field),
                    ));
                }
            };
        }

        // [0] processIdentifier
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "processIdentifier");
        let process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] initiatingDeviceIdentifier
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "initiatingDeviceIdentifier");
        let initiating_device_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [2] eventObjectIdentifier
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "eventObjectIdentifier");
        let event_object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [3] timeStamp
        let (timestamp, new_offset) = primitives::decode_timestamp(data, offset, 3)?;
        offset = new_offset;

        // [4] notificationClass
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "notificationClass");
        let notification_class = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [5] priority
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "priority");
        let priority = primitives::decode_unsigned(&data[pos..end])? as u8;
        offset = end;

        // [6] eventType
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "eventType");
        let event_type = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // Skip [7] messageText if present — scan for [8]
        while offset < data.len() {
            let (peek, peek_pos) = tags::decode_tag(data, offset)?;
            if peek.is_context(8) {
                break;
            }
            if peek.is_opening {
                // Skip the entire constructed value (opening tag ... closing tag)
                let (_, new_offset) = tags::extract_context_value(data, peek_pos, peek.number)?;
                offset = new_offset;
            } else if peek.is_closing {
                return Err(Error::decoding(
                    offset,
                    "unexpected closing tag skipping to notification-parameters",
                ));
            } else {
                let skip_end = peek_pos + peek.length as usize;
                if skip_end > data.len() {
                    return Err(Error::decoding(
                        peek_pos,
                        "EventNotification truncated skipping messageText",
                    ));
                }
                offset = skip_end;
            }
        }

        // [8] notifyType
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "notifyType");
        let notify_type = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [9] ackRequired (optional — present for ALARM/EVENT)
        let mut ack_required = false;
        if offset < data.len() {
            let (peek, peek_pos) = tags::decode_tag(data, offset)?;
            if peek.is_context(9) {
                let end = peek_pos + peek.length as usize;
                check_bounds!(peek_pos, end, "ackRequired");
                ack_required = data[peek_pos] != 0;
                offset = end;
            }
        }

        // [10] fromState
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "fromState");
        let from_state = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [11] toState
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "toState");
        let to_state = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [12] eventValues — optional
        let mut event_values = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening && peek.number == 12 {
                // Skip opening tag [12]
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                event_values = Some(NotificationParameters::decode(data, inner_start)?);
                // Find closing tag [12]
                let mut scan = inner_start;
                let mut depth: usize = 1;
                while depth > 0 && scan < data.len() {
                    let (t, next) = tags::decode_tag(data, scan)?;
                    if t.is_opening {
                        depth += 1;
                        scan = next;
                    } else if t.is_closing {
                        depth -= 1;
                        if depth == 0 {
                            offset = next;
                        } else {
                            scan = next;
                        }
                    } else {
                        let end = next.saturating_add(t.length as usize);
                        if end > data.len() {
                            return Err(Error::decoding(
                                next,
                                "EventNotification: truncated tag in eventValues",
                            ));
                        }
                        scan = end;
                    }
                }
            }
        }
        let _ = offset;

        Ok(Self {
            process_identifier,
            initiating_device_identifier,
            event_object_identifier,
            timestamp,
            notification_class,
            priority,
            event_type,
            notify_type,
            ack_required,
            from_state,
            to_state,
            event_values,
        })
    }
}

// ---------------------------------------------------------------------------
// NotificationParameters (Clause 13.5.1 — eventValues [12])
// ---------------------------------------------------------------------------

/// Notification parameter variants for eventValues (tag [12]).
#[derive(Debug, Clone, PartialEq)]
pub enum NotificationParameters {
    /// [0] Change of bitstring.
    ChangeOfBitstring {
        referenced_bitstring: (u8, Vec<u8>),
        status_flags: u8,
    },
    /// [1] Change of state.
    ChangeOfState {
        new_state: BACnetPropertyStates,
        status_flags: u8,
    },
    /// [2] Change of value.
    ChangeOfValue {
        new_value: ChangeOfValueChoice,
        status_flags: u8,
    },
    /// [3] Command failure.
    CommandFailure {
        command_value: Vec<u8>,
        status_flags: u8,
        feedback_value: Vec<u8>,
    },
    /// [4] Floating limit.
    FloatingLimit {
        reference_value: f32,
        status_flags: u8,
        setpoint_value: f32,
        error_limit: f32,
    },
    /// [5] Out of range.
    OutOfRange {
        exceeding_value: f32,
        status_flags: u8,
        deadband: f32,
        exceeded_limit: f32,
    },
    /// [8] Change of life safety.
    ChangeOfLifeSafety {
        new_state: u32,
        new_mode: u32,
        status_flags: u8,
        operation_expected: u32,
    },
    /// [9] Extended (vendor-defined).
    Extended {
        vendor_id: u16,
        extended_event_type: u32,
        parameters: Vec<u8>,
    },
    /// [10] Buffer ready.
    BufferReady {
        buffer_property: BACnetDeviceObjectPropertyReference,
        previous_notification: u32,
        current_notification: u32,
    },
    /// [11] Unsigned range.
    UnsignedRange {
        exceeding_value: u64,
        status_flags: u8,
        exceeded_limit: u64,
    },
    /// [13] Access event.
    AccessEvent {
        access_event: u32,
        status_flags: u8,
        access_event_tag: u32,
        access_event_time: (Date, Time),
        access_credential: BACnetDeviceObjectPropertyReference,
        authentication_factor: Vec<u8>,
    },
    /// [14] Double out of range.
    DoubleOutOfRange {
        exceeding_value: f64,
        status_flags: u8,
        deadband: f64,
        exceeded_limit: f64,
    },
    /// [15] Signed out of range.
    SignedOutOfRange {
        exceeding_value: i32,
        status_flags: u8,
        deadband: u64,
        exceeded_limit: i32,
    },
    /// [16] Unsigned out of range.
    UnsignedOutOfRange {
        exceeding_value: u64,
        status_flags: u8,
        deadband: u64,
        exceeded_limit: u64,
    },
    /// [17] Change of characterstring.
    ChangeOfCharacterstring {
        changed_value: String,
        status_flags: u8,
        alarm_value: String,
    },
    /// [18] Change of status flags.
    ChangeOfStatusFlags {
        present_value: Vec<u8>,
        referenced_flags: u8,
    },
    /// [19] Change of reliability.
    ChangeOfReliability {
        reliability: u32,
        status_flags: u8,
        property_values: Vec<u8>,
    },
    /// [20] None.
    NoneParams,
    /// [21] Change of discrete value.
    ChangeOfDiscreteValue {
        new_value: Vec<u8>,
        status_flags: u8,
    },
    /// [22] Change of timer.
    ChangeOfTimer {
        new_state: u32,
        status_flags: u8,
        update_time: (Date, Time),
        last_state_change: u32,
        initial_timeout: u32,
        expiration_time: (Date, Time),
    },
}

/// CHOICE within ChangeOfValue notification parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeOfValueChoice {
    ChangedBits { unused_bits: u8, data: Vec<u8> },
    ChangedValue(f32),
}

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

    /// Decode notification parameters from a position just past the opening
    /// tag of the eventValues wrapper.
    pub fn decode(data: &[u8], offset: usize) -> Result<Self, Error> {
        // Peek the inner opening tag to determine the variant
        if offset >= data.len() {
            return Err(Error::decoding(
                offset,
                "NotificationParameters: empty payload",
            ));
        }
        let (inner_tag, inner_start) = tags::decode_tag(data, offset)?;
        if !inner_tag.is_opening {
            return Err(Error::decoding(
                offset,
                "NotificationParameters: expected opening tag for variant",
            ));
        }
        let variant_tag = inner_tag.number;

        match variant_tag {
            // [1] Change of state
            1 => {
                let mut pos = inner_start;
                // [0] new-state: BACnetPropertyStates — wrapped in opening/closing [0]
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfState: expected opening tag [0] for new-state",
                    ));
                }
                pos = p;
                let new_state = decode_property_states(data, &mut pos)?;
                // Skip closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "ChangeOfState: expected closing [0]"));
                }
                pos = cp;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "ChangeOfState: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfState {
                    new_state,
                    status_flags,
                })
            }
            // [2] Change of value
            2 => {
                let mut pos = inner_start;
                // [0] new-value CHOICE — wrapped in opening/closing [0]
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfValue: expected opening [0] for new-value",
                    ));
                }
                pos = p;
                // Peek CHOICE tag
                let (choice_tag, choice_pos) = tags::decode_tag(data, pos)?;
                let new_value = if choice_tag.number == 0 {
                    // [0] changed-bits
                    let end = choice_pos + choice_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(choice_pos, "ChangeOfValue: truncated bits"));
                    }
                    let (unused, bits) = primitives::decode_bit_string(&data[choice_pos..end])?;
                    pos = end;
                    ChangeOfValueChoice::ChangedBits {
                        unused_bits: unused,
                        data: bits,
                    }
                } else {
                    // [1] changed-value (Real)
                    let end = choice_pos + choice_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(choice_pos, "ChangeOfValue: truncated real"));
                    }
                    let v = primitives::decode_real(&data[choice_pos..end])?;
                    pos = end;
                    ChangeOfValueChoice::ChangedValue(v)
                };
                // Closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "ChangeOfValue: expected closing [0]"));
                }
                pos = cp;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "ChangeOfValue: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfValue {
                    new_value,
                    status_flags,
                })
            }
            // [5] Out of range
            5 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "OutOfRange: truncated exceeding_value"));
                }
                let exceeding_value = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "OutOfRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "OutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "OutOfRange: truncated exceeded_limit"));
                }
                let exceeded_limit = primitives::decode_real(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::OutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
            }
            // [10] Buffer ready
            10 => {
                let mut pos = inner_start;
                // [0] buffer-property: BACnetDeviceObjectPropertyReference
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "BufferReady: expected opening [0] for buffer-property",
                    ));
                }
                pos = p;
                let buffer_property = decode_device_obj_prop_ref(data, &mut pos)?;
                // Closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "BufferReady: expected closing [0]"));
                }
                pos = cp;
                // [1] previous-notification
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "BufferReady: truncated previous_notification",
                    ));
                }
                let previous_notification = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [2] current-notification
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "BufferReady: truncated current_notification",
                    ));
                }
                let current_notification = primitives::decode_unsigned(&data[p..end])? as u32;
                let _ = (pos, end);
                Ok(Self::BufferReady {
                    buffer_property,
                    previous_notification,
                    current_notification,
                })
            }
            // [11] Unsigned range
            11 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "UnsignedRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_unsigned(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::UnsignedRange {
                    exceeding_value,
                    status_flags,
                    exceeded_limit,
                })
            }
            // [0] Change of bitstring
            0 => {
                let mut pos = inner_start;
                // [0] referenced-bitstring
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "ChangeOfBitstring: truncated bitstring"));
                }
                let (unused, bits) = primitives::decode_bit_string(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfBitstring: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfBitstring {
                    referenced_bitstring: (unused, bits),
                    status_flags,
                })
            }
            // [3] Command failure
            3 => {
                let mut pos = inner_start;
                // [0] command-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(pos, "CommandFailure: expected opening [0]"));
                }
                let (command_value, after) = extract_raw_context(data, p, 0)?;
                pos = after;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "CommandFailure: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] feedback-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(pos, "CommandFailure: expected opening [2]"));
                }
                let (feedback_value, _after) = extract_raw_context(data, p, 2)?;
                Ok(Self::CommandFailure {
                    command_value,
                    status_flags,
                    feedback_value,
                })
            }
            // [4] Floating limit
            4 => {
                let mut pos = inner_start;
                // [0] reference-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "FloatingLimit: truncated reference_value",
                    ));
                }
                let reference_value = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "FloatingLimit: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] setpoint-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "FloatingLimit: truncated setpoint_value",
                    ));
                }
                let setpoint_value = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [3] error-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "FloatingLimit: truncated error_limit"));
                }
                let error_limit = primitives::decode_real(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::FloatingLimit {
                    reference_value,
                    status_flags,
                    setpoint_value,
                    error_limit,
                })
            }
            // [8] Change of life safety
            8 => {
                let mut pos = inner_start;
                // [0] new-state
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfLifeSafety: truncated new_state",
                    ));
                }
                let new_state = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] new-mode
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "ChangeOfLifeSafety: truncated new_mode"));
                }
                let new_mode = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [2] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfLifeSafety: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [3] operation-expected
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfLifeSafety: truncated operation_expected",
                    ));
                }
                let operation_expected = primitives::decode_unsigned(&data[p..end])? as u32;
                let _ = (pos, end);
                Ok(Self::ChangeOfLifeSafety {
                    new_state,
                    new_mode,
                    status_flags,
                    operation_expected,
                })
            }
            // [9] Extended
            9 => {
                let mut pos = inner_start;
                // [0] vendor-id
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "Extended: truncated vendor_id"));
                }
                let vendor_id = primitives::decode_unsigned(&data[p..end])? as u16;
                pos = end;
                // [1] extended-event-type
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "Extended: truncated extended_event_type",
                    ));
                }
                let extended_event_type = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [2] parameters — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(pos, "Extended: expected opening [2]"));
                }
                let (parameters, _after) = extract_raw_context(data, p, 2)?;
                Ok(Self::Extended {
                    vendor_id,
                    extended_event_type,
                    parameters,
                })
            }
            // [13] Access event
            13 => {
                let mut pos = inner_start;
                // [0] access-event
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "AccessEvent: truncated access_event"));
                }
                let access_event = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "AccessEvent: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] access-event-tag
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "AccessEvent: truncated access_event_tag",
                    ));
                }
                let access_event_tag = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [3] access-event-time: BACnetTimeStamp (DateTime)
                let (ts, new_pos) = primitives::decode_timestamp(data, pos, 3)?;
                pos = new_pos;
                let access_event_time = match ts {
                    BACnetTimeStamp::DateTime { date, time } => (date, time),
                    _ => {
                        return Err(Error::decoding(
                            pos,
                            "AccessEvent: expected DateTime timestamp",
                        ))
                    }
                };
                // [4] access-credential: BACnetDeviceObjectPropertyReference
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 4 {
                    return Err(Error::decoding(
                        pos,
                        "AccessEvent: expected opening [4] for access-credential",
                    ));
                }
                pos = p;
                let access_credential = decode_device_obj_prop_ref(data, &mut pos)?;
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 4 {
                    return Err(Error::decoding(pos, "AccessEvent: expected closing [4]"));
                }
                pos = cp;
                // [5] authentication-factor — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 5 {
                    return Err(Error::decoding(
                        pos,
                        "AccessEvent: expected opening [5] for authentication-factor",
                    ));
                }
                let (authentication_factor, _after) = extract_raw_context(data, p, 5)?;
                Ok(Self::AccessEvent {
                    access_event,
                    status_flags,
                    access_event_tag,
                    access_event_time,
                    access_credential,
                    authentication_factor,
                })
            }
            // [14] Double out of range
            14 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "DoubleOutOfRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_double(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "DoubleOutOfRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "DoubleOutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_double(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "DoubleOutOfRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_double(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::DoubleOutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
            }
            // [15] Signed out of range
            15 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "SignedOutOfRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_signed(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "SignedOutOfRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "SignedOutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "SignedOutOfRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_signed(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::SignedOutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
            }
            // [16] Unsigned out of range
            16 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedOutOfRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "UnsignedOutOfRange: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "UnsignedOutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedOutOfRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_unsigned(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::UnsignedOutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
            }
            // [17] Change of characterstring
            17 => {
                let mut pos = inner_start;
                // [0] changed-value
                let (opt_data, new_pos) = tags::decode_optional_context(data, pos, 0)?;
                let changed_value = match opt_data {
                    Some(content) => primitives::decode_character_string(content)?,
                    None => {
                        return Err(Error::decoding(
                            pos,
                            "ChangeOfCharacterstring: missing changed_value",
                        ))
                    }
                };
                pos = new_pos;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfCharacterstring: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] alarm-value
                let (opt_data, _new_pos) = tags::decode_optional_context(data, pos, 2)?;
                let alarm_value = match opt_data {
                    Some(content) => primitives::decode_character_string(content)?,
                    None => {
                        return Err(Error::decoding(
                            pos,
                            "ChangeOfCharacterstring: missing alarm_value",
                        ))
                    }
                };
                Ok(Self::ChangeOfCharacterstring {
                    changed_value,
                    status_flags,
                    alarm_value,
                })
            }
            // [18] Change of status flags
            18 => {
                let mut pos = inner_start;
                // [0] present-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfStatusFlags: expected opening [0]",
                    ));
                }
                let (present_value, after) = extract_raw_context(data, p, 0)?;
                pos = after;
                // [1] referenced-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfStatusFlags: truncated flags",
                    ));
                }
                let referenced_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfStatusFlags {
                    present_value,
                    referenced_flags,
                })
            }
            // [19] Change of reliability
            19 => {
                let mut pos = inner_start;
                // [0] reliability
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfReliability: truncated reliability",
                    ));
                }
                let reliability = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfReliability: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] property-values — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfReliability: expected opening [2]",
                    ));
                }
                let (property_values, _after) = extract_raw_context(data, p, 2)?;
                Ok(Self::ChangeOfReliability {
                    reliability,
                    status_flags,
                    property_values,
                })
            }
            // [20] None
            20 => Ok(Self::NoneParams),
            // [21] Change of discrete value
            21 => {
                let mut pos = inner_start;
                // [0] new-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfDiscreteValue: expected opening [0]",
                    ));
                }
                let (new_value, after) = extract_raw_context(data, p, 0)?;
                pos = after;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfDiscreteValue: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfDiscreteValue {
                    new_value,
                    status_flags,
                })
            }
            // [22] Change of timer
            22 => {
                let mut pos = inner_start;
                // [0] new-state
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "ChangeOfTimer: truncated new_state"));
                }
                let new_state = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "ChangeOfTimer: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] update-time: BACnetDateTime — opening/closing [2]
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfTimer: expected opening [2] for update-time",
                    ));
                }
                pos = p;
                // Application-tagged Date
                let (d_tag, d_pos) = tags::decode_tag(data, pos)?;
                let d_end = d_pos + d_tag.length as usize;
                if d_end > data.len() {
                    return Err(Error::decoding(
                        d_pos,
                        "ChangeOfTimer: truncated update date",
                    ));
                }
                let update_date = Date::decode(&data[d_pos..d_end])?;
                pos = d_end;
                // Application-tagged Time
                let (t_tag, t_pos) = tags::decode_tag(data, pos)?;
                let t_end = t_pos + t_tag.length as usize;
                if t_end > data.len() {
                    return Err(Error::decoding(
                        t_pos,
                        "ChangeOfTimer: truncated update time",
                    ));
                }
                let update_time_val = Time::decode(&data[t_pos..t_end])?;
                pos = t_end;
                // Closing tag [2]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 2 {
                    return Err(Error::decoding(pos, "ChangeOfTimer: expected closing [2]"));
                }
                pos = cp;
                let update_time = (update_date, update_time_val);
                // [3] last-state-change
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfTimer: truncated last_state_change",
                    ));
                }
                let last_state_change = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [4] initial-timeout
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfTimer: truncated initial_timeout",
                    ));
                }
                let initial_timeout = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [5] expiration-time: BACnetDateTime — opening/closing [5]
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 5 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfTimer: expected opening [5] for expiration-time",
                    ));
                }
                pos = p;
                let (d_tag, d_pos) = tags::decode_tag(data, pos)?;
                let d_end = d_pos + d_tag.length as usize;
                if d_end > data.len() {
                    return Err(Error::decoding(
                        d_pos,
                        "ChangeOfTimer: truncated expiration date",
                    ));
                }
                let exp_date = Date::decode(&data[d_pos..d_end])?;
                pos = d_end;
                let (t_tag, t_pos) = tags::decode_tag(data, pos)?;
                let t_end = t_pos + t_tag.length as usize;
                if t_end > data.len() {
                    return Err(Error::decoding(
                        t_pos,
                        "ChangeOfTimer: truncated expiration time",
                    ));
                }
                let exp_time = Time::decode(&data[t_pos..t_end])?;
                let _ = (pos, t_end);
                let expiration_time = (exp_date, exp_time);
                Ok(Self::ChangeOfTimer {
                    new_state,
                    status_flags,
                    update_time,
                    last_state_change,
                    initial_timeout,
                    expiration_time,
                })
            }
            other => Err(Error::decoding(
                offset,
                format!("NotificationParameters variant [{other}] unknown"),
            )),
        }
    }
}

/// Encode a BACnetPropertyStates value.
fn encode_property_states(buf: &mut BytesMut, state: &BACnetPropertyStates) {
    match state {
        BACnetPropertyStates::BooleanValue(v) => {
            primitives::encode_ctx_boolean(buf, 0, *v);
        }
        BACnetPropertyStates::BinaryValue(v) => {
            primitives::encode_ctx_unsigned(buf, 1, *v as u64);
        }
        BACnetPropertyStates::EventType(v) => {
            primitives::encode_ctx_unsigned(buf, 2, *v as u64);
        }
        BACnetPropertyStates::Polarity(v) => {
            primitives::encode_ctx_unsigned(buf, 3, *v as u64);
        }
        BACnetPropertyStates::ProgramChange(v) => {
            primitives::encode_ctx_unsigned(buf, 4, *v as u64);
        }
        BACnetPropertyStates::ProgramState(v) => {
            primitives::encode_ctx_unsigned(buf, 5, *v as u64);
        }
        BACnetPropertyStates::ReasonForHalt(v) => {
            primitives::encode_ctx_unsigned(buf, 6, *v as u64);
        }
        BACnetPropertyStates::Reliability(v) => {
            primitives::encode_ctx_unsigned(buf, 7, *v as u64);
        }
        BACnetPropertyStates::State(v) => {
            primitives::encode_ctx_unsigned(buf, 8, *v as u64);
        }
        BACnetPropertyStates::SystemStatus(v) => {
            primitives::encode_ctx_unsigned(buf, 9, *v as u64);
        }
        BACnetPropertyStates::Units(v) => {
            primitives::encode_ctx_unsigned(buf, 10, *v as u64);
        }
        BACnetPropertyStates::LifeSafetyMode(v) => {
            primitives::encode_ctx_unsigned(buf, 12, *v as u64);
        }
        BACnetPropertyStates::LifeSafetyState(v) => {
            primitives::encode_ctx_unsigned(buf, 13, *v as u64);
        }
        BACnetPropertyStates::Other { tag, data } => {
            primitives::encode_ctx_octet_string(buf, *tag, data);
        }
    }
}

/// Decode BACnetPropertyStates from the current position. Advances `pos`.
fn decode_property_states(data: &[u8], pos: &mut usize) -> Result<BACnetPropertyStates, Error> {
    let (tag, content_start) = tags::decode_tag(data, *pos)?;
    let end = content_start + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(
            content_start,
            "BACnetPropertyStates: truncated",
        ));
    }
    let content = &data[content_start..end];
    *pos = end;
    match tag.number {
        0 => Ok(BACnetPropertyStates::BooleanValue(
            !content.is_empty() && content[0] != 0,
        )),
        1 => Ok(BACnetPropertyStates::BinaryValue(
            primitives::decode_unsigned(content)? as u32,
        )),
        2 => Ok(BACnetPropertyStates::EventType(
            primitives::decode_unsigned(content)? as u32,
        )),
        3 => Ok(BACnetPropertyStates::Polarity(
            primitives::decode_unsigned(content)? as u32,
        )),
        4 => Ok(BACnetPropertyStates::ProgramChange(
            primitives::decode_unsigned(content)? as u32,
        )),
        5 => Ok(BACnetPropertyStates::ProgramState(
            primitives::decode_unsigned(content)? as u32,
        )),
        6 => Ok(BACnetPropertyStates::ReasonForHalt(
            primitives::decode_unsigned(content)? as u32,
        )),
        7 => Ok(BACnetPropertyStates::Reliability(
            primitives::decode_unsigned(content)? as u32,
        )),
        8 => Ok(BACnetPropertyStates::State(
            primitives::decode_unsigned(content)? as u32,
        )),
        9 => Ok(BACnetPropertyStates::SystemStatus(
            primitives::decode_unsigned(content)? as u32,
        )),
        10 => Ok(BACnetPropertyStates::Units(
            primitives::decode_unsigned(content)? as u32,
        )),
        12 => Ok(BACnetPropertyStates::LifeSafetyMode(
            primitives::decode_unsigned(content)? as u32,
        )),
        13 => Ok(BACnetPropertyStates::LifeSafetyState(
            primitives::decode_unsigned(content)? as u32,
        )),
        n => Ok(BACnetPropertyStates::Other {
            tag: n,
            data: content.to_vec(),
        }),
    }
}

/// Decode BACnetDeviceObjectPropertyReference from context-tagged fields.
/// Expects to be positioned at the first inner field. Advances `pos` past the last field.
fn decode_device_obj_prop_ref(
    data: &[u8],
    pos: &mut usize,
) -> Result<BACnetDeviceObjectPropertyReference, Error> {
    // [0] objectIdentifier
    let (t, p) = tags::decode_tag(data, *pos)?;
    let end = p + t.length as usize;
    if end > data.len() {
        return Err(Error::decoding(
            p,
            "DeviceObjectPropertyRef: truncated objectIdentifier",
        ));
    }
    let object_identifier = ObjectIdentifier::decode(&data[p..end])?;
    *pos = end;

    // [1] propertyIdentifier
    let (t, p) = tags::decode_tag(data, *pos)?;
    let end = p + t.length as usize;
    if end > data.len() {
        return Err(Error::decoding(
            p,
            "DeviceObjectPropertyRef: truncated propertyIdentifier",
        ));
    }
    let property_identifier = primitives::decode_unsigned(&data[p..end])? as u32;
    *pos = end;

    // [2] propertyArrayIndex — optional
    let mut property_array_index = None;
    if *pos < data.len() {
        let (peek, peek_pos) = tags::decode_tag(data, *pos)?;
        if peek.is_context(2) {
            let end = peek_pos + peek.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    peek_pos,
                    "DeviceObjectPropertyRef: truncated propertyArrayIndex",
                ));
            }
            property_array_index = Some(primitives::decode_unsigned(&data[peek_pos..end])? as u32);
            *pos = end;
        }
    }

    // [3] deviceIdentifier — optional
    let mut device_identifier = None;
    if *pos < data.len() {
        let (peek, peek_pos) = tags::decode_tag(data, *pos)?;
        if peek.is_context(3) {
            let end = peek_pos + peek.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    peek_pos,
                    "DeviceObjectPropertyRef: truncated deviceIdentifier",
                ));
            }
            device_identifier = Some(ObjectIdentifier::decode(&data[peek_pos..end])?);
            *pos = end;
        }
    }

    Ok(BACnetDeviceObjectPropertyReference {
        object_identifier,
        property_identifier,
        property_array_index,
        device_identifier,
    })
}

/// Extract raw bytes between an opening and its matching closing context tag.
///
/// `start` is the position just past the opening tag. The closing tag byte for
/// context tags 0–14 is `(tag_number << 4) | 0x0F`. This scans byte-by-byte to
/// find the matching close without parsing inner content as BACnet tags.
fn extract_raw_context(
    data: &[u8],
    start: usize,
    tag_number: u8,
) -> Result<(Vec<u8>, usize), Error> {
    // For context tags < 15 the opening/closing bytes are single-byte:
    //   opening = (tag << 4) | 0x0E, closing = (tag << 4) | 0x0F
    let open_byte = (tag_number << 4) | 0x0E;
    let close_byte = (tag_number << 4) | 0x0F;
    let mut depth: usize = 1;
    let mut pos = start;
    while pos < data.len() {
        let b = data[pos];
        if b == open_byte {
            depth += 1;
        } else if b == close_byte {
            depth -= 1;
            if depth == 0 {
                let raw = data[start..pos].to_vec();
                return Ok((raw, pos + 1)); // past closing tag byte
            }
        }
        pos += 1;
    }
    Err(Error::decoding(
        start,
        format!("extract_raw_context: missing closing tag [{tag_number}]"),
    ))
}

/// Decode status flags from a bit-string content slice.
/// Returns the 4-bit status flags value.
fn decode_status_flags(data: &[u8]) -> u8 {
    // Bit string format: first byte = unused bits count, rest = data
    if data.len() >= 2 {
        let unused = data[0];
        data[1] >> (unused.min(7))
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// GetEventInformation (Clause 13.9)
// ---------------------------------------------------------------------------

/// GetEventInformation-Request — optional last_received_object_identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEventInformationRequest {
    pub last_received_object_identifier: Option<ObjectIdentifier>,
}

impl GetEventInformationRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        if let Some(ref oid) = self.last_received_object_identifier {
            primitives::encode_ctx_object_id(buf, 0, oid);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.is_empty() {
            return Ok(Self {
                last_received_object_identifier: None,
            });
        }
        let (opt_data, _) = tags::decode_optional_context(data, 0, 0)?;
        let last_received_object_identifier = if let Some(content) = opt_data {
            Some(ObjectIdentifier::decode(content)?)
        } else {
            None
        };
        Ok(Self {
            last_received_object_identifier,
        })
    }
}

/// GetEventInformation-ACK service parameters (simplified).
#[derive(Debug, Clone)]
pub struct GetEventInformationAck {
    pub list_of_event_summaries: Vec<EventSummary>,
    pub more_events: bool,
}

/// Event summary for GetEventInformation-ACK per Clause 13.9.1.2.
#[derive(Debug, Clone)]
pub struct EventSummary {
    pub object_identifier: ObjectIdentifier,
    pub event_state: u32,
    /// 3-bit bitstring: TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub acknowledged_transitions: u8,
    /// Timestamps for TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_timestamps: [BACnetTimeStamp; 3],
    /// Notify type: ALARM(0), EVENT(1), ACK_NOTIFICATION(2)
    pub notify_type: u32,
    /// 3-bit bitstring: TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_enable: u8,
    /// Priorities for TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_priorities: [u32; 3],
    pub notification_class: u32,
}

impl GetEventInformationAck {
    /// Decode a GetEventInformationAck from wire bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] listOfEventSummaries — opening tag
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(0) {
            return Err(Error::decoding(offset, "expected opening tag [0]"));
        }
        offset = pos;

        let mut list_of_event_summaries = Vec::new();

        // Parse event summaries until closing tag [0]
        loop {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(0) {
                // advance past the closing tag byte(s)
                let (_, close_pos) = tags::decode_tag(data, offset)?;
                offset = close_pos;
                break;
            }

            // [0] objectIdentifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "GetEventInfoAck truncated at oid"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] eventState
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "GetEventInfoAck truncated at eventState",
                ));
            }
            let event_state = primitives::decode_unsigned(&data[pos..end])? as u32;
            offset = end;

            // [2] acknowledgedTransitions (3-bit bitstring)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "truncated at ackedTransitions"));
            }
            // Content: [unused_bits_count, bit_data...]
            let acknowledged_transitions = if end > pos + 1 { data[pos + 1] >> 5 } else { 0 };
            offset = end;

            // [3] eventTimeStamps — opening tag
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(3) {
                return Err(Error::decoding(offset, "expected opening tag [3]"));
            }
            offset = pos;
            let mut event_timestamps = [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ];
            for ts in &mut event_timestamps {
                let (inner_tag, inner_pos) = tags::decode_tag(data, offset)?;
                if inner_tag.is_opening_tag(0) {
                    // Time choice [0] { application Time }
                    offset = inner_pos;
                    let (app_tag, app_pos) = tags::decode_tag(data, offset)?;
                    let end = app_pos + app_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(app_pos, "truncated timestamp time"));
                    }
                    *ts = BACnetTimeStamp::Time(Time::decode(&data[app_pos..end])?);
                    offset = end;
                    // closing tag [0]
                    let (_, close_pos) = tags::decode_tag(data, offset)?;
                    offset = close_pos;
                } else if inner_tag.is_context(1) {
                    // SequenceNumber choice [1]
                    let end = inner_pos + inner_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(inner_pos, "truncated timestamp seqnum"));
                    }
                    *ts = BACnetTimeStamp::SequenceNumber(primitives::decode_unsigned(
                        &data[inner_pos..end],
                    )?);
                    offset = end;
                } else if inner_tag.is_opening_tag(2) {
                    // DateTime choice [2] { Date, Time }
                    offset = inner_pos;
                    let (d_tag, d_pos) = tags::decode_tag(data, offset)?;
                    let d_end = d_pos + d_tag.length as usize;
                    if d_end > data.len() {
                        return Err(Error::decoding(d_pos, "truncated datetime date"));
                    }
                    let date = Date::decode(&data[d_pos..d_end])?;
                    offset = d_end;
                    let (t_tag, t_pos) = tags::decode_tag(data, offset)?;
                    let t_end = t_pos + t_tag.length as usize;
                    if t_end > data.len() {
                        return Err(Error::decoding(t_pos, "truncated datetime time"));
                    }
                    let time = Time::decode(&data[t_pos..t_end])?;
                    offset = t_end;
                    *ts = BACnetTimeStamp::DateTime { date, time };
                    // closing tag [2]
                    let (_, close_pos) = tags::decode_tag(data, offset)?;
                    offset = close_pos;
                } else {
                    return Err(Error::decoding(offset, "unexpected timestamp choice"));
                }
            }
            // closing tag [3]
            let (tag, _) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(3) {
                return Err(Error::decoding(offset, "expected closing tag [3]"));
            }
            let (_, close_pos) = tags::decode_tag(data, offset)?;
            offset = close_pos;

            // [4] notifyType
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "truncated at notifyType"));
            }
            let notify_type = primitives::decode_unsigned(&data[pos..end])? as u32;
            offset = end;

            // [5] eventEnable (3-bit bitstring)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "truncated at eventEnable"));
            }
            let event_enable = if end > pos + 1 { data[pos + 1] >> 5 } else { 0 };
            offset = end;

            // [6] eventPriorities — opening tag
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(6) {
                return Err(Error::decoding(offset, "expected opening tag [6]"));
            }
            offset = pos;
            let mut event_priorities = [0u32; 3];
            for pri in &mut event_priorities {
                let (tag, pos) = tags::decode_tag(data, offset)?;
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(pos, "truncated priority"));
                }
                *pri = primitives::decode_unsigned(&data[pos..end])? as u32;
                offset = end;
            }
            // closing tag [6]
            let (tag, _) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(6) {
                return Err(Error::decoding(offset, "expected closing tag [6]"));
            }
            let (_, close_pos) = tags::decode_tag(data, offset)?;
            offset = close_pos;

            list_of_event_summaries.push(EventSummary {
                object_identifier,
                event_state,
                acknowledged_transitions,
                event_timestamps,
                notify_type,
                event_enable,
                event_priorities,
                notification_class: 0, // not in wire format per Clause 13.9.1.2
            });
        }

        // [1] moreEvents
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "truncated at moreEvents"));
        }
        let more_events = data[pos] != 0;

        Ok(Self {
            list_of_event_summaries,
            more_events,
        })
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] listOfEventSummaries
        tags::encode_opening_tag(buf, 0);
        for summary in &self.list_of_event_summaries {
            // [0] objectIdentifier
            primitives::encode_ctx_object_id(buf, 0, &summary.object_identifier);
            // [1] eventState
            primitives::encode_ctx_enumerated(buf, 1, summary.event_state);
            // [2] acknowledgedTransitions (3-bit bitstring)
            primitives::encode_ctx_bit_string(buf, 2, 5, &[summary.acknowledged_transitions << 5]);
            // [3] eventTimeStamps (SEQUENCE OF 3 BACnetTimeStamp)
            tags::encode_opening_tag(buf, 3);
            for ts in &summary.event_timestamps {
                // Each timestamp is encoded as a bare CHOICE (no extra wrapping)
                // within the SEQUENCE OF
                match ts {
                    BACnetTimeStamp::Time(t) => {
                        tags::encode_opening_tag(buf, 0);
                        primitives::encode_app_time(buf, t);
                        tags::encode_closing_tag(buf, 0);
                    }
                    BACnetTimeStamp::SequenceNumber(n) => {
                        primitives::encode_ctx_unsigned(buf, 1, *n);
                    }
                    BACnetTimeStamp::DateTime { date, time } => {
                        tags::encode_opening_tag(buf, 2);
                        primitives::encode_app_date(buf, date);
                        primitives::encode_app_time(buf, time);
                        tags::encode_closing_tag(buf, 2);
                    }
                }
            }
            tags::encode_closing_tag(buf, 3);
            // [4] notifyType
            primitives::encode_ctx_enumerated(buf, 4, summary.notify_type);
            // [5] eventEnable (3-bit bitstring)
            primitives::encode_ctx_bit_string(buf, 5, 5, &[summary.event_enable << 5]);
            // [6] eventPriorities (SEQUENCE OF 3 Unsigned)
            tags::encode_opening_tag(buf, 6);
            for &p in &summary.event_priorities {
                primitives::encode_app_unsigned(buf, p as u64);
            }
            tags::encode_closing_tag(buf, 6);
        }
        tags::encode_closing_tag(buf, 0);
        // [1] moreEvents
        primitives::encode_ctx_boolean(buf, 1, self.more_events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn acknowledge_alarm_round_trip() {
        let req = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 1,
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state_acknowledged: 3, // high-limit
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = AcknowledgeAlarmRequest::decode(&buf).unwrap();
        assert_eq!(decoded.acknowledging_process_identifier, 1);
        assert_eq!(decoded.event_object_identifier, req.event_object_identifier);
        assert_eq!(decoded.event_state_acknowledged, 3);
        assert_eq!(decoded.timestamp, BACnetTimeStamp::SequenceNumber(42));
        assert_eq!(decoded.acknowledgment_source, "operator");
    }

    #[test]
    fn get_event_info_empty_request() {
        let req = GetEventInformationRequest {
            last_received_object_identifier: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = GetEventInformationRequest::decode(&buf).unwrap();
        assert!(decoded.last_received_object_identifier.is_none());
    }

    #[test]
    fn get_event_info_with_last_received() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
        let req = GetEventInformationRequest {
            last_received_object_identifier: Some(oid),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = GetEventInformationRequest::decode(&buf).unwrap();
        assert_eq!(decoded.last_received_object_identifier, Some(oid));
    }

    #[test]
    fn event_notification_round_trip() {
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();

        let req = EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device_oid,
            event_object_identifier: ai_oid,
            timestamp: BACnetTimeStamp::SequenceNumber(7),
            notification_class: 5,
            priority: 100,
            event_type: 5,  // OUT_OF_RANGE
            notify_type: 0, // ALARM
            ack_required: true,
            from_state: 0, // NORMAL
            to_state: 3,   // HIGH_LIMIT
            event_values: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();

        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        assert_eq!(decoded.process_identifier, 1);
        assert_eq!(decoded.initiating_device_identifier, device_oid);
        assert_eq!(decoded.event_object_identifier, ai_oid);
        assert_eq!(decoded.timestamp, BACnetTimeStamp::SequenceNumber(7));
        assert_eq!(decoded.notification_class, 5);
        assert_eq!(decoded.priority, 100);
        assert_eq!(decoded.event_type, 5);
        assert_eq!(decoded.notify_type, 0);
        assert!(decoded.ack_required);
        assert_eq!(decoded.from_state, 0);
        assert_eq!(decoded.to_state, 3);
        assert!(decoded.event_values.is_none());
    }

    #[test]
    fn event_notification_datetime_timestamp_round_trip() {
        use bacnet_types::primitives::{Date, Time};

        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();

        let ts = BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 2,
                day: 28,
                day_of_week: 6,
            },
            time: Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        };

        let req = EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device_oid,
            event_object_identifier: ai_oid,
            timestamp: ts.clone(),
            notification_class: 5,
            priority: 100,
            event_type: 5,
            notify_type: 0,
            ack_required: true,
            from_state: 0,
            to_state: 3,
            event_values: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();

        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        assert_eq!(decoded.timestamp, ts);
    }

    #[test]
    fn event_notification_time_timestamp_round_trip() {
        use bacnet_types::primitives::Time;

        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();

        let ts = BACnetTimeStamp::Time(Time {
            hour: 10,
            minute: 15,
            second: 30,
            hundredths: 50,
        });

        let req = EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device_oid,
            event_object_identifier: ai_oid,
            timestamp: ts.clone(),
            notification_class: 5,
            priority: 100,
            event_type: 5,
            notify_type: 0,
            ack_required: true,
            from_state: 0,
            to_state: 3,
            event_values: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();

        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        assert_eq!(decoded.timestamp, ts);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_acknowledge_alarm_empty_input() {
        assert!(AcknowledgeAlarmRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_acknowledge_alarm_truncated_1_byte() {
        let req = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 1,
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state_acknowledged: 3,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(AcknowledgeAlarmRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_acknowledge_alarm_truncated_3_bytes() {
        let req = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 1,
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state_acknowledged: 3,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(AcknowledgeAlarmRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_acknowledge_alarm_truncated_half() {
        let req = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 1,
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state_acknowledged: 3,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(AcknowledgeAlarmRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_acknowledge_alarm_invalid_tag() {
        assert!(AcknowledgeAlarmRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_event_notification_empty_input() {
        assert!(EventNotificationRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_event_notification_truncated_1_byte() {
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        let req = EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device_oid,
            event_object_identifier: ai_oid,
            timestamp: BACnetTimeStamp::SequenceNumber(7),
            notification_class: 5,
            priority: 100,
            event_type: 5,
            notify_type: 0,
            ack_required: true,
            from_state: 0,
            to_state: 3,
            event_values: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(EventNotificationRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_event_notification_truncated_3_bytes() {
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        let req = EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device_oid,
            event_object_identifier: ai_oid,
            timestamp: BACnetTimeStamp::SequenceNumber(7),
            notification_class: 5,
            priority: 100,
            event_type: 5,
            notify_type: 0,
            ack_required: true,
            from_state: 0,
            to_state: 3,
            event_values: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(EventNotificationRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_event_notification_truncated_half() {
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        let req = EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device_oid,
            event_object_identifier: ai_oid,
            timestamp: BACnetTimeStamp::SequenceNumber(7),
            notification_class: 5,
            priority: 100,
            event_type: 5,
            notify_type: 0,
            ack_required: true,
            from_state: 0,
            to_state: 3,
            event_values: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(EventNotificationRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_event_notification_invalid_tag() {
        assert!(EventNotificationRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_get_event_info_invalid_tag() {
        // Non-matching context tag — decoder treats as no last_received (lenient)
        let result = GetEventInformationRequest::decode(&[0xFF, 0xFF]).unwrap();
        assert!(result.last_received_object_identifier.is_none());
    }

    #[test]
    fn test_decode_get_event_info_truncated() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
        let req = GetEventInformationRequest {
            last_received_object_identifier: Some(oid),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(GetEventInformationRequest::decode(&buf[..1]).is_err());
    }

    // -----------------------------------------------------------------------
    // NotificationParameters round-trip tests
    // -----------------------------------------------------------------------

    /// Helper: build an EventNotificationRequest with given event_values.
    fn make_event_req(event_values: Option<NotificationParameters>) -> EventNotificationRequest {
        EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
            timestamp: BACnetTimeStamp::SequenceNumber(7),
            notification_class: 5,
            priority: 100,
            event_type: 5,
            notify_type: 0,
            ack_required: true,
            from_state: 0,
            to_state: 3,
            event_values,
        }
    }

    #[test]
    fn notification_params_out_of_range_round_trip() {
        let params = NotificationParameters::OutOfRange {
            exceeding_value: 85.5,
            status_flags: 0b1000, // IN_ALARM
            deadband: 1.0,
            exceeded_limit: 80.0,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::OutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                assert_eq!(exceeding_value, 85.5);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(deadband, 1.0);
                assert_eq!(exceeded_limit, 80.0);
            }
            other => panic!("expected OutOfRange, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_state_boolean_round_trip() {
        let params = NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::BooleanValue(true),
            status_flags: 0b1100, // IN_ALARM + FAULT
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfState {
                new_state,
                status_flags,
            } => {
                assert_eq!(new_state, BACnetPropertyStates::BooleanValue(true));
                assert_eq!(status_flags, 0b1100);
            }
            other => panic!("expected ChangeOfState, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_state_enumerated_round_trip() {
        let params = NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::State(3), // HIGH_LIMIT
            status_flags: 0b1000,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfState {
                new_state,
                status_flags,
            } => {
                assert_eq!(new_state, BACnetPropertyStates::State(3));
                assert_eq!(status_flags, 0b1000);
            }
            other => panic!("expected ChangeOfState, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_value_real_round_trip() {
        let params = NotificationParameters::ChangeOfValue {
            new_value: ChangeOfValueChoice::ChangedValue(72.5),
            status_flags: 0b0100,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfValue {
                new_value,
                status_flags,
            } => {
                assert_eq!(new_value, ChangeOfValueChoice::ChangedValue(72.5));
                assert_eq!(status_flags, 0b0100);
            }
            other => panic!("expected ChangeOfValue, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_buffer_ready_round_trip() {
        let buffer_prop = BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
            131, // LOG_BUFFER
        );
        let params = NotificationParameters::BufferReady {
            buffer_property: buffer_prop.clone(),
            previous_notification: 10,
            current_notification: 20,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::BufferReady {
                buffer_property,
                previous_notification,
                current_notification,
            } => {
                assert_eq!(buffer_property, buffer_prop);
                assert_eq!(previous_notification, 10);
                assert_eq!(current_notification, 20);
            }
            other => panic!("expected BufferReady, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_none_round_trip() {
        let params = NotificationParameters::NoneParams;
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        assert_eq!(
            decoded.event_values,
            Some(NotificationParameters::NoneParams)
        );
    }

    #[test]
    fn notification_params_unsigned_range_round_trip() {
        let params = NotificationParameters::UnsignedRange {
            exceeding_value: 500,
            status_flags: 0b1000,
            exceeded_limit: 400,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::UnsignedRange {
                exceeding_value,
                status_flags,
                exceeded_limit,
            } => {
                assert_eq!(exceeding_value, 500);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(exceeded_limit, 400);
            }
            other => panic!("expected UnsignedRange, got {:?}", other),
        }
    }

    #[test]
    fn event_notification_no_event_values_backward_compatible() {
        // Verify that event_values=None still round-trips correctly
        let req = make_event_req(None);
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        assert!(decoded.event_values.is_none());
        assert_eq!(decoded.process_identifier, 1);
        assert_eq!(decoded.to_state, 3);
    }

    #[test]
    fn get_event_information_ack_round_trip() {
        let ack = GetEventInformationAck {
            list_of_event_summaries: vec![EventSummary {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                event_state: 3,
                acknowledged_transitions: 0b101,
                event_timestamps: [
                    BACnetTimeStamp::SequenceNumber(42),
                    BACnetTimeStamp::SequenceNumber(0),
                    BACnetTimeStamp::SequenceNumber(100),
                ],
                notify_type: 0,
                event_enable: 0b111,
                event_priorities: [3, 3, 3],
                notification_class: 0,
            }],
            more_events: true,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetEventInformationAck::decode(&buf).unwrap();
        assert_eq!(decoded.list_of_event_summaries.len(), 1);
        assert!(decoded.more_events);
        let s = &decoded.list_of_event_summaries[0];
        assert_eq!(
            s.object_identifier,
            ack.list_of_event_summaries[0].object_identifier
        );
        assert_eq!(s.event_state, 3);
        assert_eq!(s.acknowledged_transitions, 0b101);
        assert_eq!(s.event_timestamps[0], BACnetTimeStamp::SequenceNumber(42));
        assert_eq!(s.notify_type, 0);
        assert_eq!(s.event_enable, 0b111);
        assert_eq!(s.event_priorities, [3, 3, 3]);
    }

    #[test]
    fn notification_params_change_of_bitstring_round_trip() {
        let params = NotificationParameters::ChangeOfBitstring {
            referenced_bitstring: (2, vec![0xA0]),
            status_flags: 0b1000,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfBitstring {
                referenced_bitstring,
                status_flags,
            } => {
                assert_eq!(referenced_bitstring, (2, vec![0xA0]));
                assert_eq!(status_flags, 0b1000);
            }
            other => panic!("expected ChangeOfBitstring, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_command_failure_round_trip() {
        let params = NotificationParameters::CommandFailure {
            command_value: vec![0x91, 0x01],
            status_flags: 0b1100,
            feedback_value: vec![0x91, 0x02],
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::CommandFailure {
                command_value,
                status_flags,
                feedback_value,
            } => {
                assert_eq!(command_value, vec![0x91, 0x01]);
                assert_eq!(status_flags, 0b1100);
                assert_eq!(feedback_value, vec![0x91, 0x02]);
            }
            other => panic!("expected CommandFailure, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_floating_limit_round_trip() {
        let params = NotificationParameters::FloatingLimit {
            reference_value: 50.0,
            status_flags: 0b1000,
            setpoint_value: 45.0,
            error_limit: 2.0,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::FloatingLimit {
                reference_value,
                status_flags,
                setpoint_value,
                error_limit,
            } => {
                assert_eq!(reference_value, 50.0);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(setpoint_value, 45.0);
                assert_eq!(error_limit, 2.0);
            }
            other => panic!("expected FloatingLimit, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_life_safety_round_trip() {
        let params = NotificationParameters::ChangeOfLifeSafety {
            new_state: 3,
            new_mode: 1,
            status_flags: 0b1000,
            operation_expected: 2,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfLifeSafety {
                new_state,
                new_mode,
                status_flags,
                operation_expected,
            } => {
                assert_eq!(new_state, 3);
                assert_eq!(new_mode, 1);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(operation_expected, 2);
            }
            other => panic!("expected ChangeOfLifeSafety, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_extended_round_trip() {
        let params = NotificationParameters::Extended {
            vendor_id: 42,
            extended_event_type: 7,
            parameters: vec![0x01, 0x02, 0x03],
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::Extended {
                vendor_id,
                extended_event_type,
                parameters,
            } => {
                assert_eq!(vendor_id, 42);
                assert_eq!(extended_event_type, 7);
                assert_eq!(parameters, vec![0x01, 0x02, 0x03]);
            }
            other => panic!("expected Extended, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_access_event_round_trip() {
        use bacnet_types::primitives::{Date, Time};

        let cred = BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap(),
            85, // PRESENT_VALUE
        );
        let params = NotificationParameters::AccessEvent {
            access_event: 5,
            status_flags: 0b1000,
            access_event_tag: 10,
            access_event_time: (
                Date {
                    year: 124,
                    month: 6,
                    day: 15,
                    day_of_week: 3,
                },
                Time {
                    hour: 10,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
            ),
            access_credential: cred.clone(),
            authentication_factor: vec![0xAB, 0xCD],
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::AccessEvent {
                access_event,
                status_flags,
                access_event_tag,
                access_event_time,
                access_credential,
                authentication_factor,
            } => {
                assert_eq!(access_event, 5);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(access_event_tag, 10);
                assert_eq!(access_event_time.0.year, 124);
                assert_eq!(access_event_time.1.hour, 10);
                assert_eq!(access_credential, cred);
                assert_eq!(authentication_factor, vec![0xAB, 0xCD]);
            }
            other => panic!("expected AccessEvent, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_double_out_of_range_round_trip() {
        let params = NotificationParameters::DoubleOutOfRange {
            exceeding_value: 100.5,
            status_flags: 0b1000,
            deadband: 0.5,
            exceeded_limit: 100.0,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::DoubleOutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                assert_eq!(exceeding_value, 100.5);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(deadband, 0.5);
                assert_eq!(exceeded_limit, 100.0);
            }
            other => panic!("expected DoubleOutOfRange, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_signed_out_of_range_round_trip() {
        let params = NotificationParameters::SignedOutOfRange {
            exceeding_value: -10,
            status_flags: 0b1000,
            deadband: 5,
            exceeded_limit: -5,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::SignedOutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                assert_eq!(exceeding_value, -10);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(deadband, 5);
                assert_eq!(exceeded_limit, -5);
            }
            other => panic!("expected SignedOutOfRange, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_unsigned_out_of_range_round_trip() {
        let params = NotificationParameters::UnsignedOutOfRange {
            exceeding_value: 200,
            status_flags: 0b1000,
            deadband: 10,
            exceeded_limit: 190,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::UnsignedOutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit,
            } => {
                assert_eq!(exceeding_value, 200);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(deadband, 10);
                assert_eq!(exceeded_limit, 190);
            }
            other => panic!("expected UnsignedOutOfRange, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_characterstring_round_trip() {
        let params = NotificationParameters::ChangeOfCharacterstring {
            changed_value: "hello".to_string(),
            status_flags: 0b1000,
            alarm_value: "alarm".to_string(),
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfCharacterstring {
                changed_value,
                status_flags,
                alarm_value,
            } => {
                assert_eq!(changed_value, "hello");
                assert_eq!(status_flags, 0b1000);
                assert_eq!(alarm_value, "alarm");
            }
            other => panic!("expected ChangeOfCharacterstring, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_status_flags_round_trip() {
        let params = NotificationParameters::ChangeOfStatusFlags {
            present_value: vec![0x91, 0x03],
            referenced_flags: 0b1010,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfStatusFlags {
                present_value,
                referenced_flags,
            } => {
                assert_eq!(present_value, vec![0x91, 0x03]);
                assert_eq!(referenced_flags, 0b1010);
            }
            other => panic!("expected ChangeOfStatusFlags, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_reliability_round_trip() {
        let params = NotificationParameters::ChangeOfReliability {
            reliability: 7,
            status_flags: 0b0100,
            property_values: vec![0x01, 0x02],
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfReliability {
                reliability,
                status_flags,
                property_values,
            } => {
                assert_eq!(reliability, 7);
                assert_eq!(status_flags, 0b0100);
                assert_eq!(property_values, vec![0x01, 0x02]);
            }
            other => panic!("expected ChangeOfReliability, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_discrete_value_round_trip() {
        let params = NotificationParameters::ChangeOfDiscreteValue {
            new_value: vec![0x91, 0x05],
            status_flags: 0b1000,
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfDiscreteValue {
                new_value,
                status_flags,
            } => {
                assert_eq!(new_value, vec![0x91, 0x05]);
                assert_eq!(status_flags, 0b1000);
            }
            other => panic!("expected ChangeOfDiscreteValue, got {:?}", other),
        }
    }

    #[test]
    fn notification_params_change_of_timer_round_trip() {
        use bacnet_types::primitives::{Date, Time};

        let params = NotificationParameters::ChangeOfTimer {
            new_state: 1,
            status_flags: 0b1000,
            update_time: (
                Date {
                    year: 124,
                    month: 3,
                    day: 10,
                    day_of_week: 1,
                },
                Time {
                    hour: 8,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
            ),
            last_state_change: 0,
            initial_timeout: 300,
            expiration_time: (
                Date {
                    year: 124,
                    month: 3,
                    day: 10,
                    day_of_week: 1,
                },
                Time {
                    hour: 8,
                    minute: 5,
                    second: 0,
                    hundredths: 0,
                },
            ),
        };
        let req = make_event_req(Some(params));
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = EventNotificationRequest::decode(&buf).unwrap();
        let ev = decoded.event_values.unwrap();
        match ev {
            NotificationParameters::ChangeOfTimer {
                new_state,
                status_flags,
                update_time,
                last_state_change,
                initial_timeout,
                expiration_time,
            } => {
                assert_eq!(new_state, 1);
                assert_eq!(status_flags, 0b1000);
                assert_eq!(update_time.0.year, 124);
                assert_eq!(update_time.1.hour, 8);
                assert_eq!(last_state_change, 0);
                assert_eq!(initial_timeout, 300);
                assert_eq!(expiration_time.0.year, 124);
                assert_eq!(expiration_time.1.minute, 5);
            }
            other => panic!("expected ChangeOfTimer, got {:?}", other),
        }
    }

    #[test]
    fn get_event_information_ack_empty_list() {
        let ack = GetEventInformationAck {
            list_of_event_summaries: vec![],
            more_events: false,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetEventInformationAck::decode(&buf).unwrap();
        assert!(decoded.list_of_event_summaries.is_empty());
        assert!(!decoded.more_events);
    }
}
