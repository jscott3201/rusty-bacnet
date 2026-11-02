//! Every served value decodes as its row's datatype and shape (#2177).
//!
//! For every row of every catalogue object (and the B/IP and B/IPv6 Network
//! Ports), the value a whole read returns is encoded as the server sends it
//! and decoded again by the row's value type: one element of the datatype
//! for a single value, any number of them for a list or an array, and
//! exactly the fixed size of a fixed array. A row whose datatype doesn't
//! match what the object serves fails here.

use super::*;
use crate::property_metadata::{ArraySize, Datatype as D, ValueShape, ValueType};
use bacnet_encoding::constructed as c;
use bacnet_encoding::primitives::{
    decode_application_value, decode_timestamp_choice, encode_property_value,
};
use bacnet_encoding::tags::{TagClass, decode_tag, extract_context_value};
use bytes::BytesMut;

/// Served values that depart from the datatype the standard gives the row.
/// Each is a bug in the object, kept here so this test still catches every
/// other row; fixing one fails the test until its entry goes. In catalogue
/// order.
const KNOWN_DEPARTURES: &[(ObjectType, PropertyIdentifier)] = &[
    // Table 12-18 gives Maintenance_Required BACnetMaintenance (an
    // enumeration); the object serves a BOOLEAN.
    (
        ObjectType::LIFE_SAFETY_POINT,
        PropertyIdentifier::MAINTENANCE_REQUIRED,
    ),
    // Table 12-61 gives Pulse_Rate Unsigned; the object serves a REAL.
    (ObjectType::ACCUMULATOR, PropertyIdentifier::PULSE_RATE),
];

#[test]
fn every_served_value_decodes_as_its_rows_value_type() {
    let mut objects = supported_representatives();
    audit_object_type_coverage(&objects);
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    let mut failures = Vec::new();
    let mut departures = Vec::new();
    let mut checked = 0;
    for object in &objects {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            let property = row.property_identifier;
            let Some(value_type) = row.effective_value_type(object_type) else {
                failures.push(format!("{object_type} {property}: no value type"));
                continue;
            };
            // A row the representative serves only once configured.
            let Ok(value) = object.read_property(property, None) else {
                continue;
            };
            let mut encoded = BytesMut::new();
            encode_property_value(&mut encoded, &value).expect("served values encode");
            checked += 1;
            if let Err(error) = decodes_as(&encoded, value_type) {
                if KNOWN_DEPARTURES.contains(&(object_type, property)) {
                    departures.push((object_type, property));
                } else {
                    failures.push(format!(
                        "{object_type} {property} as {value_type:?}: {error}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(departures, KNOWN_DEPARTURES, "a known departure was fixed");
    assert!(checked > 1000, "only {checked} values checked");
}

/// Whether `bytes` decode as a whole value of `value_type`.
fn decodes_as(bytes: &[u8], value_type: ValueType) -> Result<(), String> {
    let mut offset = 0;
    let mut count = 0u16;
    if value_type.shape == ValueShape::Single {
        offset = element_end(bytes, 0, value_type.datatype)?;
        count = 1;
    } else {
        while offset < bytes.len() {
            offset = element_end(bytes, offset, value_type.datatype)?;
            count += 1;
        }
    }
    if offset != bytes.len() {
        return Err(format!("{} octets left over", bytes.len() - offset));
    }
    match value_type.shape {
        ValueShape::Array(ArraySize::Fixed(size)) if count != size => {
            Err(format!("{count} elements in a fixed array of {size}"))
        }
        _ => Ok(()),
    }
}

/// The end of one element of `datatype` starting at `offset`.
fn element_end(bytes: &[u8], offset: usize, datatype: D) -> Result<usize, String> {
    let end = |result: Result<usize, bacnet_types::error::Error>| {
        result.map_err(|error| format!("{datatype:?} at {offset}: {error}"))
    };
    macro_rules! with {
        ($decode:path) => {
            end($decode(bytes, offset).map(|(_, end)| end))
        };
    }
    match datatype {
        D::Null
        | D::Boolean
        | D::Unsigned
        | D::Integer
        | D::Real
        | D::Double
        | D::OctetString
        | D::CharacterString
        | D::BitString
        | D::Enumerated
        | D::Date
        | D::Time
        | D::ObjectIdentifier => primitive(bytes, offset, datatype),
        D::OptionalBinaryPv | D::ProcessIdSelection => {
            either_null(bytes, offset, D::Enumerated, D::Unsigned)
        }
        D::OptionalCharacterString => either_null(bytes, offset, D::CharacterString, D::Null),
        D::OptionalPriorityFilter => either_null(bytes, offset, D::BitString, D::Null),
        D::OptionalReal | D::ClientCov => either_null(bytes, offset, D::Real, D::Null),
        D::OptionalUnsigned => either_null(bytes, offset, D::Unsigned, D::Null),
        D::DateTime => {
            let date = primitive(bytes, offset, D::Date)?;
            primitive(bytes, date, D::Time)
        }
        D::TimeStamp => with!(decode_timestamp_choice),
        D::DateRange => with!(c::decode_date_range),
        D::XyColor => with!(c::decode_xy_color),
        D::AccessRule => with!(c::decode_access_rule),
        D::ActionList => with!(c::decode_action_list),
        D::AssignedAccessRights => with!(c::decode_assigned_access_rights),
        D::AssignedLandingCalls => with!(c::decode_assigned_landing_calls),
        D::AuditLogRecord => with!(c::decode_audit_log_record_at),
        D::AuthenticationFactor => with!(c::decode_authentication_factor),
        D::AuthenticationFactorFormat => with!(c::decode_authentication_factor_format),
        D::AuthenticationPolicy => with!(c::decode_authentication_policy),
        D::BdtEntry => with!(c::decode_bdt_entry),
        D::CalendarEntry => with!(c::decode_calendar_entry),
        D::ColorCommand => with!(c::decode_color_command),
        D::CovMultipleSubscription => with!(c::decode_cov_multiple_subscription),
        D::CovSubscription => with!(c::decode_cov_subscription),
        D::CredentialAuthenticationFactor => with!(c::decode_credential_authentication_factor),
        D::DailySchedule => with!(c::decode_daily_schedule),
        D::Destination => with!(c::decode_destination),
        D::DeviceObjectPropertyReference => with!(c::decode_device_object_property_reference),
        D::DeviceObjectReference => with!(c::decode_device_object_reference),
        D::EventLogRecord => with!(c::decode_event_log_record),
        D::EventNotificationSubscription => with!(c::decode_event_notification_subscription),
        D::EventParameter => with!(c::decode_event_parameter),
        D::FaultParameter => with!(c::decode_fault_parameters),
        D::FdtEntry => with!(c::decode_fdt_entry),
        D::HostNPort => with!(c::decode_host_n_port),
        D::LandingCallStatus => with!(c::decode_landing_call_status),
        D::LandingDoorStatus => with!(c::decode_landing_door_status),
        D::LiftCarCallList => with!(c::decode_lift_car_call_list),
        D::LightingCommand => with!(c::decode_lighting_command),
        D::LogMultipleRecord => with!(c::decode_log_multiple_record),
        D::LogRecord => with!(c::decode_log_record),
        D::NameValue => with!(c::decode_name_value),
        D::ObjectPropertyReference => with!(c::decode_object_property_reference_at),
        D::PortPermission => with!(c::decode_port_permission),
        D::Prescale => with!(c::decode_prescale),
        D::PropertyAccessResult => with!(c::decode_property_access_result),
        D::ReadAccessSpecification => with!(c::decode_read_access_specification),
        D::Recipient => with!(c::decode_recipient),
        D::Scale => with!(c::decode_scale),
        // An absent reference is the empty SEQUENCE.
        D::SetpointReference if offset == bytes.len() => Ok(offset),
        D::SetpointReference => with!(c::decode_setpoint_reference_at),
        D::ShedLevel => with!(c::decode_shed_level),
        D::SpecialEvent => with!(c::decode_special_event),
        D::StageLimitValue => with!(c::decode_stage_limit_value),
        D::TimerStateChangeValue => with!(c::decode_timer_state_change_value),
        D::ValueSource => with!(c::decode_value_source),
        // CHOICEs of an application value or a context-tagged alternative.
        D::Any | D::PriorityValue | D::ChannelValue => any_element(bytes, offset),
        other => Err(format!("no decoder here for {other:?}")),
    }
}

/// One application-tagged value of a primitive `datatype`.
fn primitive(bytes: &[u8], offset: usize, datatype: D) -> Result<usize, String> {
    let (value, end) = decode_application_value(bytes, offset)
        .map_err(|error| format!("{datatype:?} at {offset}: {error}"))?;
    let matches = matches!(
        (datatype, &value),
        (D::Null, PropertyValue::Null)
            | (D::Boolean, PropertyValue::Boolean(_))
            | (D::Unsigned, PropertyValue::Unsigned(_))
            | (D::Integer, PropertyValue::Signed(_))
            | (D::Real, PropertyValue::Real(_))
            | (D::Double, PropertyValue::Double(_))
            | (D::OctetString, PropertyValue::OctetString(_))
            | (D::CharacterString, PropertyValue::CharacterString(_))
            | (D::BitString, PropertyValue::BitString { .. })
            | (D::Enumerated, PropertyValue::Enumerated(_))
            | (D::Date, PropertyValue::Date(_))
            | (D::Time, PropertyValue::Time(_))
            | (D::ObjectIdentifier, PropertyValue::ObjectIdentifier(_))
    );
    if matches {
        Ok(end)
    } else {
        Err(format!("{datatype:?} at {offset}: read {value:?}"))
    }
}

/// An application NULL, or one value of either primitive datatype.
fn either_null(bytes: &[u8], offset: usize, first: D, second: D) -> Result<usize, String> {
    primitive(bytes, offset, D::Null)
        .or_else(|_| primitive(bytes, offset, first))
        .or_else(|_| primitive(bytes, offset, second))
}

/// One well-framed element: an application value, or a context-tagged one,
/// primitive or constructed.
fn any_element(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let (tag, start) = decode_tag(bytes, offset).map_err(|error| error.to_string())?;
    if tag.class == TagClass::Application {
        return decode_application_value(bytes, offset)
            .map(|(_, end)| end)
            .map_err(|error| error.to_string());
    }
    if tag.is_opening {
        return extract_context_value(bytes, start, tag.number)
            .map(|(_, end)| end)
            .map_err(|error| error.to_string());
    }
    if tag.is_closing {
        return Err(format!("closing tag at {offset}"));
    }
    Ok(start + tag.length as usize)
}
