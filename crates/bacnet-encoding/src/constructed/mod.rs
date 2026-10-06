//! Full ASN.1 framing codecs for constructed Clause-21 types whose property
//! values are CHOICE/SEQUENCE productions — context-tagged framing per
//! ASHRAE 135-2020 Clause 20.2.1.5/20.2.1.6.
//!
//! These codecs encode to and decode from raw application-layer bytes, the
//! same bytes carried by [`bacnet_types::primitives::PropertyValue::ApplicationData`].
//! Objects serve framed properties (e.g. `Event_Parameters`,
//! `Fault_Parameters`, `Recipient_List`) by encoding with these functions;
//! the flat application-tagged model in [`crate::primitives::encode_property_value`]
//! cannot express their wire form.
//!
//! Tag-form rules applied here:
//!
//! - A CHOICE alternative over a `SEQUENCE` is an opening/closing context tag
//!   pair around the SEQUENCE's members.
//! - A CHOICE alternative over a primitive base type is a context-specific
//!   primitive tag holding the raw contents (e.g. `none [0] NULL`).
//! - A context-tagged inner `CHOICE` is always explicitly tagged: an
//!   opening/closing pair around the alternative's own encoding.
//! - Inner members declared `[n] T` over primitive `T` are context-tagged
//!   primitives; inner `SEQUENCE OF` / embedded `SEQUENCE` members are
//!   constructed opening/closing pairs holding application-tagged elements.

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetExtendedPropertyState, BACnetPropertyStates,
    BACnetProprietaryPropertyState,
};
use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::primitives;
use crate::tags::{self, TagClass};
use tagged::{
    contents, decode_ctx_object_id, decode_ctx_unsigned, decode_optional_ctx, expect_closing,
    expect_end, expect_opening,
};

pub mod access_credential;
pub mod access_rule;
mod action_list;
pub mod assigned_landing_calls;
mod audit_notification;
mod audit_record;
pub mod authentication_policy;
pub mod calendar;
mod channel_value;
mod color_command;
pub mod cov_subscription;
mod event_log_record;
mod event_notification;
pub mod event_notification_subscription;
pub mod event_parameter;
pub mod fault_parameter;
mod floor_pairs;
pub mod landing_call_status;
pub mod landing_door_status;
pub mod lift_car_call_list;
mod lighting_command;
mod log_fields;
mod log_multiple_record;
mod log_record;
mod members;
mod name_value;
pub mod object_property_reference;
pub mod port_permission;
mod property_access_result;
mod property_value;
mod read_access;
pub mod recipient;
mod scale;
pub mod schedule;
mod shed_level;
pub mod staging;
pub mod tagged;
mod value_source;

pub use access_credential::{
    decode_assigned_access_rights, decode_authentication_factor,
    decode_authentication_factor_format, decode_credential_authentication_factor,
    encode_assigned_access_rights, encode_authentication_factor,
    encode_authentication_factor_format, encode_credential_authentication_factor,
};
pub use access_rule::{decode_access_rule, encode_access_rule};
pub use action_list::{
    decode_action_command, decode_action_list, encode_action_command, encode_action_list,
};
pub use assigned_landing_calls::{decode_assigned_landing_calls, encode_assigned_landing_calls};
pub use audit_notification::{decode_audit_notification_at, encode_audit_notification};
pub use audit_record::{
    decode_audit_log_record, decode_audit_log_record_at, decode_audit_log_record_result_at,
    encode_audit_log_record, encode_audit_log_record_result,
};
pub use authentication_policy::{decode_authentication_policy, encode_authentication_policy};
pub use calendar::{
    decode_calendar_entry, decode_calendar_entry_list, decode_date_range, encode_calendar_entry,
    encode_calendar_entry_list, encode_date_range,
};
pub use channel_value::{channel_value_end, constructed_channel_value, ConstructedChannelValue};
pub use color_command::{
    decode_color_command, decode_color_command_value, decode_xy_color, encode_color_command,
    encode_xy_color,
};
pub use cov_subscription::{
    decode_cov_multiple_subscription, decode_cov_subscription, encode_cov_multiple_subscription,
    encode_cov_multiple_subscription_list, encode_cov_subscription, encode_cov_subscription_list,
};
pub use event_log_record::{decode_event_log_record, encode_event_log_record};
pub use event_notification::{
    decode_event_notification, decode_event_notification_tolerant, decode_notification_parameters,
    encode_event_notification, encode_notification_parameters,
};
pub use event_notification_subscription::{
    decode_event_notification_subscription, encode_event_notification_subscription,
    encode_event_notification_subscription_list,
};
pub use event_parameter::{decode_event_parameter, encode_event_parameter};
pub use fault_parameter::{decode_fault_parameters, encode_fault_parameters};
pub use landing_call_status::{
    decode_landing_call_status, decode_landing_call_status_list, encode_landing_call_status,
    encode_landing_call_status_list,
};
pub use landing_door_status::{decode_landing_door_status, encode_landing_door_status};
pub use lift_car_call_list::{decode_lift_car_call_list, encode_lift_car_call_list};
pub use lighting_command::{
    decode_lighting_command, decode_lighting_command_value, encode_lighting_command,
};
pub use log_multiple_record::{decode_log_multiple_record, encode_log_multiple_record};
pub use log_record::{decode_log_record, encode_log_record};
pub use name_value::{decode_name_value, encode_name_value};
pub use object_property_reference::{
    decode_object_property_reference, decode_object_property_reference_at,
    decode_setpoint_reference, decode_setpoint_reference_at, encode_object_property_reference,
    encode_setpoint_reference,
};
pub use port_permission::{decode_port_permission, encode_port_permission};
pub use property_access_result::{decode_property_access_result, encode_property_access_result};
pub use property_value::{
    decode_bacnet_property_value, decode_bacnet_property_value_in_list,
    decode_bacnet_property_value_in_list_detailed, encode_bacnet_property_value,
    extract_property_value, PropertyValueBoundary, PropertyValueDecodeError,
    PropertyValueDecodeFailure, PropertyValueDecodeStage,
};
pub use read_access::{
    decode_property_reference, decode_read_access_specification, encode_property_reference,
    encode_read_access_specification,
};
pub use recipient::{
    check_decoded_mac_len, check_encoded_mac_len, decode_destination, decode_destination_list,
    decode_recipient, encode_destination, encode_destination_list, encode_recipient,
};
pub use scale::{decode_prescale, decode_scale, encode_prescale, encode_scale};
pub use schedule::{
    decode_daily_schedule, decode_exception_schedule, decode_special_event,
    decode_special_event_period, decode_time_value, decode_weekly_schedule, encode_daily_schedule,
    encode_exception_schedule, encode_special_event, encode_special_event_period,
    encode_time_value, encode_weekly_schedule,
};
pub use shed_level::{decode_shed_level, encode_shed_level};
pub use staging::{
    decode_device_object_reference, decode_stage_limit_value, encode_device_object_reference,
    encode_stage_limit_value,
};

pub use value_source::{decode_value_source, encode_value_source};

/// Upper bound on decoded SEQUENCE OF / list lengths, mirroring the socket-
/// facing posture of `bacnet-services`' `MAX_DECODED_ITEMS`. Prevents memory
/// exhaustion from malformed framed payloads.
const MAX_FRAMED_ITEMS: usize = 10_000;

// ---------------------------------------------------------------------------
// BACnetPropertyStates (Clause 21 CHOICE) — spec-tagged framing
// ---------------------------------------------------------------------------

/// Encode a [`BACnetPropertyStates`] using the Standard 135-2020 Clause 21 tags.
///
/// Returns an error without modifying `buf` when a constructed proprietary
/// value does not contain a BACnet TLV sequence.
pub fn encode_property_state(
    buf: &mut BytesMut,
    state: &BACnetPropertyStates,
) -> Result<(), Error> {
    use BACnetPropertyStates as S;
    if let S::Other(value) = state {
        if value.is_constructed() {
            validate_tlv_sequence(value.data(), "proprietary property-state body")?;
            let mut framed = BytesMut::new();
            tags::encode_opening_tag(&mut framed, value.tag());
            let (_, body_start) = tags::decode_tag(&framed, 0)?;
            framed.extend_from_slice(value.data());
            tags::encode_closing_tag(&mut framed, value.tag());
            let (_, end) = tags::extract_context_value(&framed, body_start, value.tag())?;
            expect_end(&framed, end, end, "proprietary property-state body")?;
        }
    }
    match state {
        S::BooleanValue(v) => primitives::encode_ctx_boolean(buf, 0, *v),
        S::BinaryValue(v) => primitives::encode_ctx_enumerated(buf, 1, *v),
        S::EventType(v) => primitives::encode_ctx_enumerated(buf, 2, *v),
        S::Polarity(v) => primitives::encode_ctx_enumerated(buf, 3, *v),
        S::ProgramChange(v) => primitives::encode_ctx_enumerated(buf, 4, *v),
        S::ProgramState(v) => primitives::encode_ctx_enumerated(buf, 5, *v),
        S::ReasonForHalt(v) => primitives::encode_ctx_enumerated(buf, 6, *v),
        S::Reliability(v) => primitives::encode_ctx_enumerated(buf, 7, *v),
        S::State(v) => primitives::encode_ctx_enumerated(buf, 8, *v),
        S::SystemStatus(v) => primitives::encode_ctx_enumerated(buf, 9, *v),
        S::Units(v) => primitives::encode_ctx_enumerated(buf, 10, *v),
        S::UnsignedValue(v) => primitives::encode_ctx_unsigned(buf, 11, *v as u64),
        S::LifeSafetyMode(v) => primitives::encode_ctx_enumerated(buf, 12, *v),
        S::LifeSafetyState(v) => primitives::encode_ctx_enumerated(buf, 13, *v),
        S::RestartReason(v) => primitives::encode_ctx_enumerated(buf, 14, *v),
        S::DoorAlarmState(v) => primitives::encode_ctx_enumerated(buf, 15, *v),
        S::Action(v) => primitives::encode_ctx_enumerated(buf, 16, *v),
        S::DoorSecuredStatus(v) => primitives::encode_ctx_enumerated(buf, 17, *v),
        S::DoorStatus(v) => primitives::encode_ctx_enumerated(buf, 18, *v),
        S::DoorValue(v) => primitives::encode_ctx_enumerated(buf, 19, *v),
        S::FileAccessMethod(v) => primitives::encode_ctx_enumerated(buf, 20, *v),
        S::LockStatus(v) => primitives::encode_ctx_enumerated(buf, 21, *v),
        S::LifeSafetyOperation(v) => primitives::encode_ctx_enumerated(buf, 22, *v),
        S::Maintenance(v) => primitives::encode_ctx_enumerated(buf, 23, *v),
        S::NodeType(v) => primitives::encode_ctx_enumerated(buf, 24, *v),
        S::NotifyType(v) => primitives::encode_ctx_enumerated(buf, 25, *v),
        S::ShedState(v) => primitives::encode_ctx_enumerated(buf, 27, *v),
        S::SilencedState(v) => primitives::encode_ctx_enumerated(buf, 28, *v),
        S::AccessEvent(v) => primitives::encode_ctx_enumerated(buf, 30, *v),
        S::ZoneOccupancyState(v) => primitives::encode_ctx_enumerated(buf, 31, *v),
        S::AccessCredentialDisableReason(v) => primitives::encode_ctx_enumerated(buf, 32, *v),
        S::AccessCredentialDisable(v) => primitives::encode_ctx_enumerated(buf, 33, *v),
        S::AuthenticationStatus(v) => primitives::encode_ctx_enumerated(buf, 34, *v),
        S::BackupState(v) => primitives::encode_ctx_enumerated(buf, 36, *v),
        S::WriteStatus(v) => primitives::encode_ctx_enumerated(buf, 37, *v),
        S::LightingInProgress(v) => primitives::encode_ctx_enumerated(buf, 38, *v),
        S::LightingOperation(v) => primitives::encode_ctx_enumerated(buf, 39, *v),
        S::LightingTransition(v) => primitives::encode_ctx_enumerated(buf, 40, *v),
        S::IntegerValue(v) => primitives::encode_ctx_signed(buf, 41, *v),
        S::BinaryLightingValue(v) => primitives::encode_ctx_enumerated(buf, 42, *v),
        S::TimerState(v) => primitives::encode_ctx_enumerated(buf, 43, *v),
        S::TimerTransition(v) => primitives::encode_ctx_enumerated(buf, 44, *v),
        S::BacnetIpMode(v) => primitives::encode_ctx_enumerated(buf, 45, *v),
        S::NetworkPortCommand(v) => primitives::encode_ctx_enumerated(buf, 46, *v),
        S::NetworkType(v) => primitives::encode_ctx_enumerated(buf, 47, *v),
        S::NetworkNumberQuality(v) => primitives::encode_ctx_enumerated(buf, 48, *v),
        S::EscalatorOperationDirection(v) => primitives::encode_ctx_enumerated(buf, 49, *v),
        S::EscalatorFault(v) => primitives::encode_ctx_enumerated(buf, 50, *v),
        S::EscalatorMode(v) => primitives::encode_ctx_enumerated(buf, 51, *v),
        S::LiftCarDirection(v) => primitives::encode_ctx_enumerated(buf, 52, *v),
        S::LiftCarDoorCommand(v) => primitives::encode_ctx_enumerated(buf, 53, *v),
        S::LiftCarDriveStatus(v) => primitives::encode_ctx_enumerated(buf, 54, *v),
        S::LiftCarMode(v) => primitives::encode_ctx_enumerated(buf, 55, *v),
        S::LiftGroupMode(v) => primitives::encode_ctx_enumerated(buf, 56, *v),
        S::LiftFault(v) => primitives::encode_ctx_enumerated(buf, 57, *v),
        S::ProtocolLevel(v) => primitives::encode_ctx_enumerated(buf, 58, *v),
        S::AuditLevel(v) => primitives::encode_ctx_enumerated(buf, 59, *v),
        S::AuditOperation(v) => primitives::encode_ctx_enumerated(buf, 60, *v),
        S::ExtendedValue(v) => primitives::encode_ctx_unsigned(buf, 63, v.encoded() as u64),
        S::Other(v) if v.is_constructed() => {
            tags::encode_opening_tag(buf, v.tag());
            buf.extend_from_slice(v.data());
            tags::encode_closing_tag(buf, v.tag());
        }
        S::Other(v) => primitives::encode_ctx_octet_string(buf, v.tag(), v.data()),
    }
    Ok(())
}

/// Decode one [`BACnetPropertyStates`] CHOICE element at `offset`.
///
/// Proprietary tags 64 through 254 retain their encoded contents in
/// [`BACnetPropertyStates::Other`]. Reserved standard tags are rejected.
pub fn decode_property_state(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetPropertyStates, usize), Error> {
    use BACnetPropertyStates as S;
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Context || tag.is_closing {
        return Err(tagged::misplaced_tag(
            data,
            &tag,
            None,
            offset,
            "BACnetPropertyStates: expected a context tag",
        ));
    }
    if tag.is_opening {
        if !(64..=254).contains(&tag.number) {
            return Err(Error::invalid_tag(
                offset,
                "BACnetPropertyStates: constructed form requires a proprietary tag",
            ));
        }
        let (content, end) = tags::extract_context_value(data, pos, tag.number)?;
        validate_tlv_sequence(content, "proprietary property-state body")?;
        return Ok((
            S::Other(BACnetProprietaryPropertyState::constructed(
                tag.number,
                content.to_vec(),
            )?),
            end,
        ));
    }
    let (content, end) = contents(data, pos, tag.length)?;
    let unsigned = || -> Result<u32, Error> {
        u32::try_from(primitives::decode_unsigned(content)?)
            .map_err(|_| Error::out_of_range(pos, "BACnetPropertyStates: contents exceed u32"))
    };
    let state = match tag.number {
        0 => {
            if content.len() != 1 {
                return Err(Error::decoding(
                    offset,
                    format!(
                        "BACnetPropertyStates boolean-value: expected 1 contents octet, got {}",
                        content.len()
                    ),
                ));
            }
            match content[0] {
                0 => S::BooleanValue(false),
                1 => S::BooleanValue(true),
                value => {
                    return Err(Error::decoding(
                        pos,
                        format!("BACnetPropertyStates boolean-value must be 0 or 1, got {value}"),
                    ));
                }
            }
        }
        1 => S::BinaryValue(unsigned()?),
        2 => S::EventType(unsigned()?),
        3 => S::Polarity(unsigned()?),
        4 => S::ProgramChange(unsigned()?),
        5 => S::ProgramState(unsigned()?),
        6 => S::ReasonForHalt(unsigned()?),
        7 => S::Reliability(unsigned()?),
        8 => S::State(unsigned()?),
        9 => S::SystemStatus(unsigned()?),
        10 => S::Units(unsigned()?),
        11 => S::UnsignedValue(unsigned()?),
        12 => S::LifeSafetyMode(unsigned()?),
        13 => S::LifeSafetyState(unsigned()?),
        14 => S::RestartReason(unsigned()?),
        15 => S::DoorAlarmState(unsigned()?),
        16 => S::Action(unsigned()?),
        17 => S::DoorSecuredStatus(unsigned()?),
        18 => S::DoorStatus(unsigned()?),
        19 => S::DoorValue(unsigned()?),
        20 => S::FileAccessMethod(unsigned()?),
        21 => S::LockStatus(unsigned()?),
        22 => S::LifeSafetyOperation(unsigned()?),
        23 => S::Maintenance(unsigned()?),
        24 => S::NodeType(unsigned()?),
        25 => S::NotifyType(unsigned()?),
        27 => S::ShedState(unsigned()?),
        28 => S::SilencedState(unsigned()?),
        30 => S::AccessEvent(unsigned()?),
        31 => S::ZoneOccupancyState(unsigned()?),
        32 => S::AccessCredentialDisableReason(unsigned()?),
        33 => S::AccessCredentialDisable(unsigned()?),
        34 => S::AuthenticationStatus(unsigned()?),
        36 => S::BackupState(unsigned()?),
        37 => S::WriteStatus(unsigned()?),
        38 => S::LightingInProgress(unsigned()?),
        39 => S::LightingOperation(unsigned()?),
        40 => S::LightingTransition(unsigned()?),
        41 => S::IntegerValue(primitives::decode_signed_canonical(content)?),
        42 => S::BinaryLightingValue(unsigned()?),
        43 => S::TimerState(unsigned()?),
        44 => S::TimerTransition(unsigned()?),
        45 => S::BacnetIpMode(unsigned()?),
        46 => S::NetworkPortCommand(unsigned()?),
        47 => S::NetworkType(unsigned()?),
        48 => S::NetworkNumberQuality(unsigned()?),
        49 => S::EscalatorOperationDirection(unsigned()?),
        50 => S::EscalatorFault(unsigned()?),
        51 => S::EscalatorMode(unsigned()?),
        52 => S::LiftCarDirection(unsigned()?),
        53 => S::LiftCarDoorCommand(unsigned()?),
        54 => S::LiftCarDriveStatus(unsigned()?),
        55 => S::LiftCarMode(unsigned()?),
        56 => S::LiftGroupMode(unsigned()?),
        57 => S::LiftFault(unsigned()?),
        58 => S::ProtocolLevel(unsigned()?),
        59 => S::AuditLevel(unsigned()?),
        60 => S::AuditOperation(unsigned()?),
        63 => S::ExtendedValue(BACnetExtendedPropertyState::from_encoded(unsigned()?)?),
        other @ 64..=254 => S::Other(BACnetProprietaryPropertyState::primitive(
            other,
            content.to_vec(),
        )?),
        reserved => {
            return Err(Error::invalid_tag(
                offset,
                format!("BACnetPropertyStates context tag {reserved} is reserved"),
            ));
        }
    };
    Ok((state, end))
}

// ---------------------------------------------------------------------------
// BACnetDeviceObjectPropertyReference (Clause 21 SEQUENCE)
// ---------------------------------------------------------------------------

/// Encode the context-tagged members of a `BACnetDeviceObjectPropertyReference`:
/// `object-identifier [0]`, `property-identifier [1]`, optional
/// `property-array-index [2]`, optional `device-identifier [3]` — the body
/// that sits between the enclosing field's opening/closing tags.
pub(crate) fn encode_dopr_body(buf: &mut BytesMut, r: &BACnetDeviceObjectPropertyReference) {
    primitives::encode_ctx_object_id(buf, 0, &r.object_identifier);
    primitives::encode_ctx_unsigned(buf, 1, r.property_identifier as u64);
    if let Some(index) = r.property_array_index {
        primitives::encode_ctx_unsigned(buf, 2, index as u64);
    }
    if let Some(ref device) = r.device_identifier {
        primitives::encode_ctx_object_id(buf, 3, device);
    }
}

/// Decode the members written by [`encode_dopr_body`], stopping before the
/// enclosing closing tag.
pub(crate) fn decode_dopr_body(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(BACnetDeviceObjectPropertyReference, usize), Error> {
    let (object_identifier, offset) = decode_ctx_object_id(data, offset, 0, what)?;
    let (property_identifier, offset) = decode_ctx_unsigned::<u32>(data, offset, 1, what)?;
    let (property_array_index, offset) =
        decode_optional_ctx(data, offset, 2, what, decode_ctx_unsigned::<u32>)?;
    let (device_identifier, offset) =
        decode_optional_ctx(data, offset, 3, what, decode_ctx_object_id)?;
    Ok((
        BACnetDeviceObjectPropertyReference {
            object_identifier,
            property_identifier,
            property_array_index,
            device_identifier,
        },
        offset,
    ))
}

/// Encode one bare `BACnetDeviceObjectPropertyReference`, the counterpart of
/// [`decode_device_object_property_reference`]: its members with no enclosing
/// frame, so a BACnetLIST of these references is their encodings back to back.
pub fn encode_device_object_property_reference(
    buf: &mut BytesMut,
    r: &BACnetDeviceObjectPropertyReference,
) {
    encode_dopr_body(buf, r);
}

/// Decode one bare `BACnetDeviceObjectPropertyReference` at `offset`; returns
/// it and the offset past its last member. A BACnetLIST of these references
/// concatenates its elements with no frame, so walking the list calls this at
/// each element's start.
pub fn decode_device_object_property_reference(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetDeviceObjectPropertyReference, usize), Error> {
    decode_dopr_body(data, offset, "BACnetDeviceObjectPropertyReference")
}

/// Validate a BACnet TLV sequence without normalizing its encoded values.
///
/// This checks matching context tags, the context nesting limit, and
/// application-value forms while preserving defined CharacterString encodings.
pub fn validate_tlv_sequence(data: &[u8], what: &str) -> Result<(), Error> {
    let mut offset = 0;
    let mut count = 0;
    while offset < data.len() {
        if count >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                offset,
                format!("{what}: sequence exceeds item limit"),
            ));
        }
        let (tag, content) = tags::decode_tag(data, offset)?;
        if tag.is_opening {
            let (inner, next) = tags::extract_context_value(data, content, tag.number)?;
            validate_tlv_sequence(inner, what)?;
            offset = next;
        } else if tag.class == TagClass::Application {
            offset = primitives::validate_application_value(data, offset)?;
        } else {
            let (_, next) = primitives::decode_application_value(data, offset)?;
            offset = next;
        }
        count += 1;
    }
    Ok(())
}

/// Validate the shared Extended Event/Fault `parameters` production.
/// Its only context-tagged CHOICE is `reference [0]` over
/// `BACnetDeviceObjectPropertyReference`; NotificationParameters uses a
/// different Extended production with `property-value [0]`.
pub(crate) fn validate_extended_parameters(data: &[u8], what: &str) -> Result<(), Error> {
    let mut offset = 0;
    let mut count = 0;
    while offset < data.len() {
        if count >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                offset,
                format!("{what}: parameters exceed item limit"),
            ));
        }
        let (tag, _) = tags::decode_tag(data, offset)?;
        if tag.class == TagClass::Context {
            let content = expect_opening(data, offset, 0, what)?;
            let (_, after_reference) = decode_dopr_body(data, content, what)?;
            offset = expect_closing(data, after_reference, 0, what)?;
        } else {
            offset = primitives::validate_application_value(data, offset)?;
        }
        count += 1;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
