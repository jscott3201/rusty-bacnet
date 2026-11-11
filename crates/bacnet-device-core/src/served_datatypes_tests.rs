//! Every served value decodes as its row's datatype and shape (#2177).
//!
//! The catalogue's objects (with the B/IP and B/IPv6 Network Ports) are
//! first given a member in each list and array they leave empty, written
//! through the WriteProperty value path ([]),
//! since an empty value decodes as any datatype. Then every row's whole read
//! is encoded as the server sends it and decoded again by the row's value
//! type: one element of the datatype for a single value, any number for a
//! list or an array, and exactly the size of a fixed array. A row whose
//! datatype doesn't match what the object serves fails here.

use bacnet_encoding::constructed as c;
use bacnet_encoding::primitives::{
    decode_application_value, decode_timestamp_choice, encode_property_value,
};
use bacnet_encoding::tags::{TagClass, decode_tag, extract_context_value};
use bacnet_objects::access_control::{
    AccessCredentialObject, AccessDoorObject, AccessPointObject, AccessUserObject,
    AccessZoneObject, CredentialDataInputObject,
};
use bacnet_objects::command::CommandObject;
use bacnet_objects::elevator::ElevatorGroupObject;
use bacnet_objects::group::{GlobalGroupObject, GroupObject, StructuredViewObject};
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_objects::load_control::LoadControlObject;
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::property_metadata::{ArraySize, Datatype as D, ValueShape, ValueType};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetActionCommand, BACnetActionList, BACnetAssignedAccessRights,
    BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat, BACnetCalendarEntry,
    BACnetCredentialAuthenticationFactor, BACnetDestination, BACnetDeviceObjectPropertyReference,
    BACnetDeviceObjectReference, BACnetRecipient, BACnetSpecialEvent, BACnetTimeValue,
    PropertyReference, ReadAccessSpecification, SpecialEventPeriod,
};
use bacnet_types::enums::{
    AccessAuthenticationFactorDisable, AccessCredentialDisableReason, AccessRuleLocationSpecifier,
    AccessRuleTimeRangeSpecifier, AuthenticationFactorType, ObjectType as T,
    PropertyIdentifier as P,
};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

/// Served values that depart from the datatype the standard gives the row.
/// Each is a bug in the object, kept here so this test still catches every
/// other row; fixing one fails the test until its entry goes. In catalogue
/// order.
const KNOWN_DEPARTURES: &[(T, P)] = &[];

/// Rows this test can't check: values still empty after [] (an
/// empty value decodes as any datatype) or unreadable whole. In catalogue
/// order.
const BLIND_SPOTS: &[(T, P)] = &[
    // The running server fills these from its own state.
    (T::DEVICE, P::DEVICE_ADDRESS_BINDING),
    (T::DEVICE, P::ACTIVE_COV_SUBSCRIPTIONS),
    (T::DEVICE, P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS),
    // Read from the members' objects, which need a database.
    (T::GROUP, P::PRESENT_VALUE),
    // An absent reference: the empty SEQUENCE is its whole encoding.
    (T::LOOP, P::SETPOINT_REFERENCE),
    // A whole read of a log buffer is refused; ReadRange reads it.
    (T::TREND_LOG, P::LOG_BUFFER),
    (T::EVENT_LOG, P::LOG_BUFFER),
    // Names follow the members, set with no names, and have no setter.
    (T::GLOBAL_GROUP, P::GROUP_MEMBER_NAMES),
    (T::TREND_LOG_MULTIPLE, P::LOG_BUFFER),
    // A write needs the device's clock.
    (T::TREND_LOG_MULTIPLE, P::LOG_DEVICE_OBJECT_PROPERTY),
    // Filled by AddListElement through the server's list handler.
    (T::NOTIFICATION_FORWARDER, P::SUBSCRIBED_RECIPIENTS),
    // The object fills these as cars and faults report.
    (T::ELEVATOR_GROUP, P::LANDING_CALLS),
    (T::ESCALATOR, P::FAULT_SIGNALS),
    (T::LIFT, P::FAULT_SIGNALS),
    (T::AUDIT_LOG, P::LOG_BUFFER),
];

fn oid(object_type: T, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Application-tagged values back to back.
fn app(values: &[PropertyValue]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for value in values {
        encode_property_value(&mut buf, value).unwrap();
    }
    buf.to_vec()
}

fn dopr(object_type: T) -> Vec<u8> {
    let mut buf = BytesMut::new();
    c::encode_device_object_property_reference(
        &mut buf,
        &BACnetDeviceObjectPropertyReference {
            object_identifier: oid(object_type, 1),
            property_identifier: P::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    );
    buf.to_vec()
}

/// The whole value written to each list or array the catalogue leaves empty.
fn populations() -> Vec<(T, P, Vec<u8>)> {
    let date = Date {
        year: 126,
        month: 1,
        day: 1,
        day_of_week: 4,
    };
    let time = |hour| Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let mut destination = BytesMut::new();
    c::encode_destination(
        &mut destination,
        &BACnetDestination {
            valid_days: bacnet_types::bitstring::DaysOfWeek::all(),
            from_time: time(0),
            to_time: time(23),
            recipient: BACnetRecipient::Device(oid(T::DEVICE, 7)),
            process_identifier: 1,
            issue_confirmed_notifications: false,
            transitions: bacnet_types::bitstring::EventTransitionBits::all(),
        },
    )
    .unwrap();
    let mut calendar = BytesMut::new();
    c::encode_calendar_entry(&mut calendar, &BACnetCalendarEntry::Date(date));
    let mut exception = BytesMut::new();
    c::encode_special_event(
        &mut exception,
        &BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(date)),
            list_of_time_values: vec![BACnetTimeValue {
                time: time(8),
                value: PropertyValue::Null,
            }],
            event_priority: 16,
        },
    )
    .unwrap();
    let mut rule = BytesMut::new();
    c::encode_access_rule(
        &mut rule,
        &BACnetAccessRule {
            time_range_specifier: AccessRuleTimeRangeSpecifier::ALWAYS,
            time_range: None,
            location_specifier: AccessRuleLocationSpecifier::ALL,
            location: None,
            enable: true,
        },
    );
    let unsigned = app(&[PropertyValue::Unsigned(2)]);
    let enumerated = app(&[PropertyValue::Enumerated(1)]);
    vec![
        (T::CALENDAR, P::DATE_LIST, calendar.to_vec()),
        (T::MULTI_STATE_INPUT, P::ALARM_VALUES, unsigned.clone()),
        (
            T::NOTIFICATION_CLASS,
            P::RECIPIENT_LIST,
            destination.to_vec(),
        ),
        (
            T::SCHEDULE,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            dopr(T::ANALOG_VALUE),
        ),
        (T::MULTI_STATE_VALUE, P::ALARM_VALUES, unsigned),
        (T::ACCESS_DOOR, P::MASKED_ALARM_VALUES, enumerated.clone()),
        (T::ACCESS_DOOR, P::ALARM_VALUES, enumerated.clone()),
        (T::ACCESS_DOOR, P::FAULT_VALUES, enumerated.clone()),
        (T::ACCESS_ZONE, P::ALARM_VALUES, enumerated),
        (
            T::NOTIFICATION_FORWARDER,
            P::RECIPIENT_LIST,
            destination.to_vec(),
        ),
        (
            T::CHANNEL,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            dopr(T::ANALOG_VALUE),
        ),
        (
            T::CHANNEL,
            P::EXECUTION_DELAY,
            app(&[PropertyValue::Unsigned(0)]),
        ),
        (T::SCHEDULE, P::EXCEPTION_SCHEDULE, exception.to_vec()),
        (T::ACCESS_RIGHTS, P::POSITIVE_ACCESS_RULES, rule.to_vec()),
        (T::ACCESS_RIGHTS, P::NEGATIVE_ACCESS_RULES, rule.to_vec()),
    ]
}

/// Objects whose empty lists and arrays have no write route, built with
/// their setters instead, replacing the catalogue's.
fn configured() -> Vec<Box<dyn BACnetObject>> {
    let reference = |object_type| BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: oid(object_type, 1),
    };
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![BACnetActionCommand {
                device_identifier: None,
                object_identifier: oid(T::ANALOG_VALUE, 1),
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Real(1.0),
                priority: Some(8),
                post_delay: None,
                quit_on_failure: false,
                write_successful: false,
            }],
        }])
        .unwrap();
    let mut group = GroupObject::new(1, "GRP-1").unwrap();
    group
        .add_member(ReadAccessSpecification {
            object_identifier: oid(T::ANALOG_INPUT, 1),
            list_of_property_references: vec![PropertyReference {
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
            }],
        })
        .unwrap();
    let mut global = GlobalGroupObject::new(1, "GG-1").unwrap();
    global
        .add_group_member(BACnetDeviceObjectPropertyReference {
            object_identifier: oid(T::ANALOG_INPUT, 1),
            property_identifier: P::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        })
        .unwrap();
    let mut view = StructuredViewObject::new(1, "SV-1").unwrap();
    view.add_subordinate(reference(T::ANALOG_INPUT), "a")
        .unwrap();
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.add_member(reference(T::LIFE_SAFETY_ZONE)).unwrap();
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.add_zone_member(reference(T::LIFE_SAFETY_POINT))
        .unwrap();
    zone.add_member(oid(T::LIFE_SAFETY_ZONE, 2)).unwrap();
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_door_members([reference(T::ACCESS_POINT)]).unwrap();
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .add_disable_reason(AccessCredentialDisableReason::DISABLED_LOCKOUT)
        .unwrap();
    credential
        .set_authentication_factors(vec![BACnetCredentialAuthenticationFactor {
            disable: AccessAuthenticationFactorDisable::NONE,
            authentication_factor: BACnetAuthenticationFactor {
                format_type: AuthenticationFactorType::SIMPLE_NUMBER16,
                format_class: 0,
                value: vec![0, 1],
            },
        }])
        .unwrap();
    credential
        .set_assigned_access_rights(vec![BACnetAssignedAccessRights {
            assigned_access_rights: reference(T::ACCESS_RIGHTS),
            enable: true,
        }])
        .unwrap();
    let mut input = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    input
        .set_supported_formats([(
            BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::SIMPLE_NUMBER16),
            1,
        )])
        .unwrap();
    let mut access_point = AccessPointObject::new(1, "AP-1").unwrap();
    access_point
        .set_access_doors([reference(T::ACCESS_DOOR)])
        .unwrap();
    let mut user = AccessUserObject::new(1, "USER-1").unwrap();
    user.set_credentials([reference(T::ACCESS_CREDENTIAL)])
        .unwrap();
    user.set_members([oid(T::ACCESS_USER, 2)]).unwrap();
    user.set_member_of([oid(T::ACCESS_USER, 3)]).unwrap();
    let mut access_zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    access_zone
        .set_entry_points([reference(T::ACCESS_POINT)])
        .unwrap();
    access_zone
        .set_exit_points([reference(T::ACCESS_POINT)])
        .unwrap();
    let mut elevators = ElevatorGroupObject::new(1, "EG-1").unwrap();
    elevators.add_member(oid(T::LIFT, 1));
    // Clients may rewrite the shed levels but not size them.
    let mut load_control = LoadControlObject::new(1, "LC-1").unwrap();
    load_control.set_shed_levels(vec![(1, "a".into())]);
    vec![
        Box::new(command),
        Box::new(group),
        Box::new(global),
        Box::new(view),
        Box::new(point),
        Box::new(zone),
        Box::new(door),
        Box::new(credential),
        Box::new(input),
        Box::new(access_point),
        Box::new(user),
        Box::new(access_zone),
        Box::new(elevators),
        Box::new(load_control),
    ]
}

/// Write each population through the WriteProperty value path, and Staging's
/// startup Present_Value, as the server does.
fn populate(objects: &mut [Box<dyn BACnetObject>]) -> Vec<String> {
    let mut failures = Vec::new();
    for object in objects
        .iter_mut()
        .filter(|object| object.object_identifier().object_type() == T::STAGING)
    {
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(5.0), None)
            .unwrap();
    }
    for (object_type, property, bytes) in populations() {
        let object = objects
            .iter_mut()
            .find(|object| object.object_identifier().object_type() == object_type)
            .unwrap();
        let written = crate::write::gate_and_decode_write(&**object, property, None, &bytes)
            .and_then(|value| object.write_property(property, None, value, None));
        if let Err(error) = written {
            failures.push(format!("populating {object_type} {property}: {error:?}"));
        }
    }
    failures
}

fn is_empty(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::List(items) => items.is_empty(),
        PropertyValue::ApplicationData(bytes) => bytes.is_empty(),
        _ => false,
    }
}

#[test]
fn every_served_value_decodes_as_its_rows_value_type() {
    let mut objects = bacnet_objects::catalogue::representatives();
    assert_eq!(objects.len(), 64, "one object of each catalogue type");
    for replacement in configured() {
        let object_type = replacement.object_identifier().object_type();
        let slot = objects
            .iter_mut()
            .find(|object| object.object_identifier().object_type() == object_type)
            .unwrap();
        *slot = replacement;
    }
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    let mut failures = populate(&mut objects);
    let mut departures = Vec::new();
    let mut blind = Vec::new();
    let mut checked = 0;
    for object in &objects {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            let property = row.property_identifier;
            let Some(value_type) = row.effective_value_type(object_type) else {
                failures.push(format!("{object_type} {property}: no value type"));
                continue;
            };
            let value = match object.read_property(property, None) {
                Ok(value) if !is_empty(&value) => value,
                _ => {
                    blind.push((object_type, property));
                    continue;
                }
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
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
    assert_eq!(departures, KNOWN_DEPARTURES, "a known departure was fixed");
    assert_eq!(blind, BLIND_SPOTS, "the unchecked rows changed");
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
        D::OptionalBinaryPv => or_null(bytes, offset, D::Enumerated),
        D::ProcessIdSelection | D::OptionalUnsigned => or_null(bytes, offset, D::Unsigned),
        D::OptionalCharacterString => or_null(bytes, offset, D::CharacterString),
        D::OptionalPriorityFilter => or_null(bytes, offset, D::BitString),
        D::OptionalReal | D::ClientCov => or_null(bytes, offset, D::Real),
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

/// An application NULL, or one value of the primitive datatype.
fn or_null(bytes: &[u8], offset: usize, datatype: D) -> Result<usize, String> {
    primitive(bytes, offset, D::Null).or_else(|_| primitive(bytes, offset, datatype))
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
