//! The form masks (#2524): an encoder limited to some forms encodes those
//! as the full encoder does and refuses the rest, leaving the buffer as it
//! was.

use super::*;
use bacnet_types::constructed::OpaqueChoice;

const OUT_OF_RANGE_AND_STATE: u32 =
    notification_forms(&[EventType::OUT_OF_RANGE, EventType::CHANGE_OF_STATE]);

fn out_of_range() -> NotificationParameters {
    NotificationParameters::OutOfRange {
        exceeding_value: 80.0,
        status_flags: StatusFlags::IN_ALARM,
        deadband: 1.0,
        exceeded_limit: 75.0,
    }
}

fn floating_limit() -> NotificationParameters {
    NotificationParameters::FloatingLimit {
        reference_value: 50.0,
        status_flags: StatusFlags::IN_ALARM,
        setpoint_value: 45.0,
        error_limit: 2.0,
    }
}

fn alarm(event_values: Option<NotificationParameters>) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values,
    }
}

#[test]
fn a_mask_has_a_bit_per_form_at_its_event_type() {
    assert_eq!(OUT_OF_RANGE_AND_STATE, 1 << 5 | 1 << 1);
    assert_eq!(notification_forms(&[]), 0);
    // No form of its own past 31: nothing to take.
    assert_eq!(notification_forms(&[EventType::from_raw(40)]), 0);
}

#[test]
fn a_form_the_mask_takes_encodes_as_every_form_does() {
    let states = NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::BinaryValue(1),
        status_flags: StatusFlags::IN_ALARM,
    };
    for params in [out_of_range(), states] {
        let (mut every, mut limited) = (BytesMut::new(), BytesMut::new());
        encode_notification_parameters(&params, &mut every).unwrap();
        encode_notification_parameters_of::<OUT_OF_RANGE_AND_STATE>(&params, &mut limited).unwrap();
        assert_eq!(limited, every, "{params:?}");
    }
    let (mut every, mut limited) = (BytesMut::new(), BytesMut::new());
    encode_event_notification(&alarm(Some(out_of_range())), &mut every).unwrap();
    encode_event_notification_of::<OUT_OF_RANGE_AND_STATE>(
        &alarm(Some(out_of_range())),
        &mut limited,
    )
    .unwrap();
    assert_eq!(limited, every);
}

#[test]
fn a_form_the_mask_leaves_out_is_refused_with_the_buffer_unchanged() {
    let mut buf = BytesMut::from(&[0xAA][..]);
    assert!(
        encode_notification_parameters_of::<OUT_OF_RANGE_AND_STATE>(&floating_limit(), &mut buf)
            .is_err()
    );
    assert!(
        encode_event_notification_of::<OUT_OF_RANGE_AND_STATE>(
            &alarm(Some(floating_limit())),
            &mut buf
        )
        .is_err()
    );
    assert_eq!(buf.as_ref(), [0xAA]);
    // An opaque form only with every form.
    let opaque = NotificationParameters::Opaque(OpaqueChoice {
        tag: 7,
        constructed: true,
        data: vec![],
    });
    assert!(encode_notification_parameters_of::<{ u32::MAX >> 1 }>(&opaque, &mut buf).is_err());
    encode_notification_parameters_of::<ALL_NOTIFICATION_FORMS>(&opaque, &mut buf).unwrap();
    assert_eq!(buf.as_ref(), [0xAA, 0x7E, 0x7F]);
}

#[test]
fn a_request_without_event_values_needs_no_form() {
    let (mut every, mut none) = (BytesMut::new(), BytesMut::new());
    encode_event_notification(&alarm(None), &mut every).unwrap();
    encode_event_notification_of::<0>(&alarm(None), &mut none).unwrap();
    assert_eq!(none, every);
}
