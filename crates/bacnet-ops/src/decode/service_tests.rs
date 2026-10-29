//! Named service parameters in the decode tree (#2074): a snapshot of the
//! parameters node for each service, from frames the stack's own encoders
//! build, and the fallbacks.

use bacnet_encoding::apdu::{Apdu, ComplexAck, ConfirmedRequest, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::constructed::encode_event_notification;
use bacnet_services::alarm_event::{
    EventNotificationRequest, EventSummary, GetEventInformationAck, GetEventInformationRequest,
    NotificationParameters,
};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{
    ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetPropertyValue, PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    ConfirmedServiceChoice as Confirmed, EnableDisable, ErrorClass, ErrorCode, EventState,
    EventType, NotifyType, ObjectType, PropertyIdentifier as P, ReinitializedState,
    UnconfirmedServiceChoice as Unconfirmed,
};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, StatusFlags};
use bytes::{Bytes, BytesMut};

use super::super::{Field, decode_apdu_tree, decode_tree};
use crate::decode::frame::Framing;
use crate::decode::testing::bip;

const REAL_72_5: [u8; 5] = [0x44, 0x42, 0x91, 0x00, 0x00];
/// Status flags with none set.
const NO_FLAGS: [u8; 3] = [0x82, 0x04, 0x00];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn encoded(apdu: &Apdu) -> Vec<u8> {
    let mut out = BytesMut::new();
    encode_apdu(&mut out, apdu).unwrap();
    out.to_vec()
}

fn request(service: Confirmed, params: BytesMut) -> Vec<u8> {
    encoded(&Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: params.freeze(),
    }))
}

fn unconfirmed(service: Unconfirmed, params: BytesMut) -> Vec<u8> {
    encoded(&Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: service,
        service_request: params.freeze(),
    }))
}

fn ack(service: Confirmed, params: BytesMut) -> Vec<u8> {
    encoded(&Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_ack: params.freeze(),
    }))
}

/// The parameters node of a decoded APDU.
fn parameters(apdu: &[u8]) -> Field {
    decode_apdu_tree(apdu)
        .children
        .pop()
        .filter(|field| field.label.ends_with("parameters"))
        .expect("a parameters node")
}

/// A field and its parts, one a line, indented by depth: `label @range`,
/// with `!` after a field marked as device text.
fn render(field: &Field, depth: usize, out: &mut String) {
    let mark = if field.device_text { " !" } else { "" };
    out.push_str(&format!(
        "{}{} @{}..{}{mark}\n",
        "  ".repeat(depth),
        field.label,
        field.range.start,
        field.range.end
    ));
    for child in &field.children {
        render(child, depth + 1, out);
    }
}

pub(super) fn snapshot(apdu: &[u8]) -> String {
    let mut out = String::new();
    render(&parameters(apdu), 0, &mut out);
    out
}

#[track_caller]
fn assert_snapshot(apdu: &[u8], expected: &str) {
    let got = snapshot(apdu);
    assert_eq!(got.trim_end(), expected.trim(), "\n{got}");
}

fn rp_request(index: Option<u32>) -> Vec<u8> {
    let mut params = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_VALUE, 2),
        property_identifier: P::PRIORITY_ARRAY,
        property_array_index: index,
    }
    .encode(&mut params);
    request(Confirmed::READ_PROPERTY, params)
}

fn rp_ack(object: ObjectIdentifier, property: P, value: &[u8]) -> Vec<u8> {
    let mut params = BytesMut::new();
    ReadPropertyACK {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut params);
    ack(Confirmed::READ_PROPERTY, params)
}

#[test]
fn read_property_and_its_ack_are_named() {
    assert_snapshot(
        &rp_request(Some(8)),
        "
read-property parameters @4..13
  Object: analog-value 2 @4..9
  Property: priority-array @9..11
  Array index: 8 @11..13",
    );
    // A binary value's present value is named by its state, from the
    // object type and property.
    let frame = rp_ack(
        oid(ObjectType::BINARY_VALUE, 3),
        P::PRESENT_VALUE,
        &[0x91, 0x01],
    );
    assert_snapshot(
        &frame,
        "
read-property parameters @3..14
  Object: binary-value 3 @3..8
  Property: present-value @8..10
  Value: active @10..14",
    );
    // A constructed value keeps its tags under it.
    let mut value = REAL_72_5.to_vec();
    value.extend_from_slice(&REAL_72_5);
    let frame = rp_ack(oid(ObjectType::ANALOG_VALUE, 1), P::PRIORITY_ARRAY, &value);
    assert_snapshot(
        &frame,
        "
read-property parameters @3..22
  Object: analog-value 1 @3..8
  Property: priority-array @8..10
  Value: [72.5, 72.5] @10..22
    Real: 72.5 @11..16
    Real: 72.5 @16..21",
    );
}

#[test]
fn a_read_property_multiple_and_its_ack_are_named() {
    let mut params = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: P::PRESENT_VALUE,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: P::PRIORITY_ARRAY,
                    property_array_index: Some(3),
                },
            ],
        }],
    }
    .encode(&mut params)
    .unwrap();
    assert_snapshot(
        &request(Confirmed::READ_PROPERTY_MULTIPLE, params),
        "
read-property-multiple parameters @4..17
  Read analog-input 1 @4..17
    Object: analog-input 1 @4..9
    Properties @9..17
      Property: present-value @10..12
      Property: priority-array[3] @12..16",
    );

    let mut params = BytesMut::new();
    ReadPropertyMultipleACK {
        list_of_read_access_results: vec![ReadAccessResult {
            object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_results: vec![
                ReadResultElement {
                    property_identifier: P::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(REAL_72_5.to_vec()),
                    error: None,
                },
                ReadResultElement {
                    property_identifier: P::STATUS_FLAGS,
                    property_array_index: None,
                    property_value: Some(NO_FLAGS.to_vec()),
                    error: None,
                },
                ReadResultElement {
                    property_identifier: P::DESCRIPTION,
                    property_array_index: None,
                    property_value: None,
                    error: Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
                },
            ],
        }],
    }
    .encode(&mut params);
    assert_snapshot(
        &ack(Confirmed::READ_PROPERTY_MULTIPLE, params),
        "
read-property-multiple parameters @3..34
  Results for analog-input 1 @3..34
    Object: analog-input 1 @3..8
    Properties @8..34
      present-value: 72.5 @9..18
        Property: present-value @9..11
        Value: 72.5 @11..18
      status-flags: none @18..25
        Property: status-flags @18..20
        Value: none @20..25
      description: error property:unknown-property @25..33
        Property: description @25..27
        Error: property:unknown-property @27..33",
    );
}

#[test]
fn a_write_property_is_named() {
    let mut params = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_OUTPUT, 1),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
        property_value: REAL_72_5.to_vec(),
        priority: Some(8),
    }
    .encode(&mut params)
    .unwrap();
    assert_snapshot(
        &request(Confirmed::WRITE_PROPERTY, params),
        "
write-property parameters @4..20
  Object: analog-output 1 @4..9
  Property: present-value @9..11
  Value: 72.5 @11..18
  Priority: 8 @18..20",
    );
}

fn cov_params() -> BytesMut {
    let mut params = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 7,
        initiating_device_identifier: oid(ObjectType::DEVICE, 5),
        monitored_object_identifier: oid(ObjectType::BINARY_INPUT, 1),
        time_remaining: 300,
        list_of_values: vec![
            BACnetPropertyValue {
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x91, 0x00],
                priority: None,
            },
            BACnetPropertyValue {
                property_identifier: P::STATUS_FLAGS,
                property_array_index: None,
                value: NO_FLAGS.to_vec(),
                priority: None,
            },
        ],
    }
    .encode(&mut params);
    params
}

#[test]
fn cov_notifications_are_named_both_ways() {
    let confirmed = request(Confirmed::CONFIRMED_COV_NOTIFICATION, cov_params());
    assert_snapshot(
        &confirmed,
        "
confirmed-cov-notification parameters @4..34
  Subscriber process ID: 7 @4..6
  Initiating device: device 5 @6..11
  Monitored object: binary-input 1 @11..16
  Time remaining: 300 s @16..19
  Values @19..34
    present-value: inactive @20..26
      Property: present-value @20..22
      Value: inactive @22..26
    status-flags: none @26..33
      Property: status-flags @26..28
      Value: none @28..33",
    );
    // The unconfirmed one names the same fields, three octets earlier.
    let unconfirmed = unconfirmed(Unconfirmed::UNCONFIRMED_COV_NOTIFICATION, cov_params());
    let labels = |field: &Field| {
        let mut out = String::new();
        render(field, 0, &mut out);
        out.lines()
            .map(|line| line.split(" @").next().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(&parameters(&confirmed))[1..],
        labels(&parameters(&unconfirmed))[1..]
    );
}

fn event_params(message: Option<&str>) -> BytesMut {
    let mut params = BytesMut::new();
    encode_event_notification(
        &EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: oid(ObjectType::DEVICE, 5),
            event_object_identifier: oid(ObjectType::ANALOG_INPUT, 2),
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            notification_class: 10,
            priority: 100,
            event_type: EventType::OUT_OF_RANGE,
            message_text: message.map(str::to_string),
            notify_type: NotifyType::ALARM,
            ack_required: true,
            from_state: EventState::NORMAL,
            to_state: EventState::HIGH_LIMIT,
            event_values: Some(NotificationParameters::OutOfRange {
                exceeding_value: 101.5,
                status_flags: StatusFlags::IN_ALARM,
                deadband: 1.0,
                exceeded_limit: 100.0,
            }),
        },
        &mut params,
    )
    .unwrap();
    params
}

#[test]
fn event_notifications_are_named_both_ways() {
    let frame = request(
        Confirmed::CONFIRMED_EVENT_NOTIFICATION,
        event_params(Some("Zone hot")),
    );
    assert_snapshot(
        &frame,
        r#"
confirmed-event-notification parameters @4..67
  Process ID: 1 @4..6
  Initiating device: device 5 @6..11
  Event object: analog-input 2 @11..16
  Time stamp: seq 42 @16..20
  Notification class: 10 @20..22
  Priority: 100 @22..24
  Event type: out-of-range @24..26
  Message text: "Zone hot" @26..37 !
  Notify type: alarm @37..39
  Ack required: true @39..41
  From state: normal @41..43
  To state: high-limit @43..45
  Event values: out-of-range: exceeding value 101.5, flags in-alarm, deadband 1, limit 100 @45..67 !
    [5] opening @46..66
      [0] 42CB0000 (1120600064) @47..52
      [1] 0480 (1152) @52..55
      [2] 3F800000 (1065353216) @55..60
      [3] 42C80000 (1120403456) @60..65"#,
    );
    let frame = unconfirmed(
        Unconfirmed::UNCONFIRMED_EVENT_NOTIFICATION,
        event_params(None),
    );
    assert_snapshot(
        &frame,
        "
unconfirmed-event-notification parameters @2..54
  Process ID: 1 @2..4
  Initiating device: device 5 @4..9
  Event object: analog-input 2 @9..14
  Time stamp: seq 42 @14..18
  Notification class: 10 @18..20
  Priority: 100 @20..22
  Event type: out-of-range @22..24
  Notify type: alarm @24..26
  Ack required: true @26..28
  From state: normal @28..30
  To state: high-limit @30..32
  Event values: out-of-range: exceeding value 101.5, flags in-alarm, deadband 1, limit 100 @32..54 !
    [5] opening @33..53
      [0] 42CB0000 (1120600064) @34..39
      [1] 0480 (1152) @39..42
      [2] 3F800000 (1065353216) @42..47
      [3] 42C80000 (1120403456) @47..52",
    );
}

#[test]
fn get_event_information_and_its_ack_are_named() {
    let mut params = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier: Some(oid(ObjectType::ANALOG_INPUT, 3)),
    }
    .encode(&mut params);
    assert_snapshot(
        &request(Confirmed::GET_EVENT_INFORMATION, params),
        "
get-event-information parameters @4..9
  Last received object: analog-input 3 @4..9",
    );

    let mut params = BytesMut::new();
    GetEventInformationAck {
        list_of_event_summaries: vec![EventSummary {
            object_identifier: oid(ObjectType::ANALOG_INPUT, 2),
            event_state: EventState::HIGH_LIMIT,
            acknowledged_transitions: EventTransitionBits::TO_FAULT
                | EventTransitionBits::TO_NORMAL,
            event_timestamps: [
                BACnetTimeStamp::SequenceNumber(42),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(7),
            ],
            notify_type: NotifyType::ALARM,
            event_enable: EventTransitionBits::all(),
            event_priorities: [3, 5, 200],
        }],
        more_events: false,
    }
    .encode(&mut params)
    .unwrap();
    assert_snapshot(
        &ack(Confirmed::GET_EVENT_INFORMATION, params),
        "
get-event-information parameters @3..38
  Event summaries: 1 summary @3..36
    analog-input 2: high-limit @4..35
      Object: analog-input 2 @4..9
      Event state: high-limit @9..11
      Acked transitions: to-fault, to-normal @11..14
      Event time stamps: to-offnormal seq 42, to-fault seq 0, to-normal seq 7 @14..22
      Notify type: alarm @22..24
      Event enable: to-offnormal, to-fault, to-normal @24..27
      Event priorities: to-offnormal 3, to-fault 5, to-normal 200 @27..35
  More events: false @36..38",
    );
}

/// A password is hidden on the named path, and on the tag walk when the
/// decoder refuses the request (a password past 20 characters here).
#[test]
fn passwords_are_hidden_on_both_paths() {
    let mut params = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: Some(60),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: Some("Zq7-hunter2".into()),
    }
    .encode(&mut params)
    .unwrap();
    let frame = request(Confirmed::DEVICE_COMMUNICATION_CONTROL, params);
    assert_snapshot(
        &frame,
        "
device-communication-control parameters @4..22
  Time duration: 60 min @4..6
  Enable/disable: disable-initiation @6..8
  Password: (hidden) @8..22",
    );

    let mut params = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: Some("Zq7-hunter2".into()),
    }
    .encode(&mut params)
    .unwrap();
    assert_snapshot(
        &request(Confirmed::REINITIALIZE_DEVICE, params),
        "
reinitialize-device parameters @4..20
  Reinitialized state: warmstart @4..6
  Password: (hidden) @6..20",
    );

    // 24 characters: the decoder refuses it.
    let long = b"Zq7-hunter2-Zq7-hunter2-";
    let mut params = vec![0x19, 0x00];
    params.extend_from_slice(&[0x2D, u8::try_from(long.len() + 1).unwrap(), 0x00]);
    params.extend_from_slice(long);
    let frame = request(
        Confirmed::DEVICE_COMMUNICATION_CONTROL,
        BytesMut::from(&params[..]),
    );
    let text = snapshot(&frame);
    assert!(!text.contains("68756E746572"), "{text}");
    assert_snapshot(
        &frame,
        "
device-communication-control parameters @4..33
  [1] 00 (0) @4..6
  Password: (hidden) @6..33
  Not read as named fields: decoding error at offset 2: DCC password exceeds 20 characters @4..33",
    );
}

/// A frame cut off mid-value falls back to the tag walk, says why, and
/// still covers every octet.
#[test]
fn a_damaged_frame_falls_back_to_the_tag_walk() {
    let mut frame = rp_ack(
        oid(ObjectType::ANALOG_INPUT, 1),
        P::PRESENT_VALUE,
        &REAL_72_5,
    );
    frame.truncate(frame.len() - 3);
    let node = parameters(&frame);
    assert_eq!(node.range.end, frame.len());
    assert_snapshot(
        &frame,
        "
read-property parameters @3..14
  [0] 00000001 (1) @3..8
  [1] 55 (85) @8..10
  [3] opening @10..14
    Application tag 4: buffer too short: need 16 bytes, have 14 @11..14
  Not read as named fields: buffer too short: need 13 bytes, have 11 @3..14",
    );
}

/// A segment holds only part of the parameters: it is walked tag by tag.
#[test]
fn a_segment_is_walked_tag_by_tag() {
    let mut params = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut params);
    let frame = encoded(&Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id: 1,
        sequence_number: Some(0),
        proposed_window_size: Some(4),
        service_choice: Confirmed::READ_PROPERTY,
        service_ack: Bytes::from(params.to_vec()),
    }));
    assert_snapshot(
        &frame,
        "
read-property parameters @5..12
  [0] 00000001 (1) @5..10
  [1] 55 (85) @10..12",
    );
}

/// Through a whole B/IP frame the ranges count from the frame's start, and
/// a character string value is device text.
#[test]
fn a_frame_names_its_fields_with_frame_offsets() {
    let apdu = rp_ack(
        oid(ObjectType::ANALOG_INPUT, 1),
        P::DESCRIPTION,
        &[0x74, 0x00, b'F', b'a', b'n'],
    );
    let mut npdu = vec![0x01, 0x00];
    npdu.extend_from_slice(&apdu);
    let frame = bip(&npdu, false);
    let fields = decode_tree(Framing::Bip, &frame);
    let mut out = String::new();
    render(fields.last().unwrap().children.last().unwrap(), 0, &mut out);
    assert_eq!(
        out.trim_end(),
        r#"read-property parameters @9..23
  Object: analog-input 1 @9..14
  Property: description @14..16
  Value: "Fan" @16..23 !"#
    );
}
