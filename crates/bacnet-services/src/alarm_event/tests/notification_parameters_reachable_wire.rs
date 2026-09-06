use super::*;

fn assert_literal(params: NotificationParameters, literal: &[u8]) {
    let mut encoded = BytesMut::new();
    params.encode(&mut encoded).unwrap();
    assert_eq!(encoded.as_ref(), literal);
    assert_eq!(NotificationParameters::decode(literal, 0).unwrap(), params);
}

#[test]
fn reachable_notification_parameter_alternatives_have_exact_literal_bytes() {
    assert_literal(
        NotificationParameters::ChangeOfBitstring {
            referenced_bitstring: (5, vec![0xe0]),
            status_flags: 0b1010,
        },
        &[0x0e, 0x0a, 0x05, 0xe0, 0x1a, 0x04, 0xa0, 0x0f],
    );
    assert_literal(
        NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::BinaryValue(1),
            status_flags: 0b1000,
        },
        &[0x1e, 0x0e, 0x19, 0x01, 0x0f, 0x1a, 0x04, 0x80, 0x1f],
    );
    assert_literal(
        NotificationParameters::ChangeOfValue {
            new_value: ChangeOfValueChoice::ChangedValue(12.5),
            status_flags: 0b0100,
        },
        &[
            0x2e, 0x0e, 0x1c, 0x41, 0x48, 0x00, 0x00, 0x0f, 0x1a, 0x04, 0x40, 0x2f,
        ],
    );
    assert_literal(
        NotificationParameters::ChangeOfValue {
            new_value: ChangeOfValueChoice::ChangedBits {
                unused_bits: 5,
                data: vec![0xa0],
            },
            status_flags: 0b0010,
        },
        &[0x2e, 0x0e, 0x0a, 0x05, 0xa0, 0x0f, 0x1a, 0x04, 0x20, 0x2f],
    );
    assert_literal(
        NotificationParameters::CommandFailure {
            command_value: vec![0x91, 0x01],
            status_flags: 0b1100,
            feedback_value: vec![0x91, 0x00],
        },
        &[
            0x3e, 0x0e, 0x91, 0x01, 0x0f, 0x1a, 0x04, 0xc0, 0x2e, 0x91, 0x00, 0x2f, 0x3f,
        ],
    );
    assert_literal(
        NotificationParameters::FloatingLimit {
            reference_value: 50.0,
            status_flags: 0b1000,
            setpoint_value: 45.0,
            error_limit: 2.0,
        },
        &[
            0x4e, 0x0c, 0x42, 0x48, 0x00, 0x00, 0x1a, 0x04, 0x80, 0x2c, 0x42, 0x34, 0x00, 0x00,
            0x3c, 0x40, 0x00, 0x00, 0x00, 0x4f,
        ],
    );
    assert_literal(
        NotificationParameters::OutOfRange {
            exceeding_value: 85.0,
            status_flags: 0b1000,
            deadband: 2.0,
            exceeded_limit: 80.0,
        },
        &[
            0x5e, 0x0c, 0x42, 0xaa, 0x00, 0x00, 0x1a, 0x04, 0x80, 0x2c, 0x40, 0x00, 0x00, 0x00,
            0x3c, 0x42, 0xa0, 0x00, 0x00, 0x5f,
        ],
    );
    assert_literal(
        NotificationParameters::ChangeOfReliability {
            reliability: 2,
            status_flags: 0b1100,
            property_values: vec![0x09, 0x55, 0x2e, 0x44, 0x3f, 0x80, 0x00, 0x00, 0x2f],
        },
        &[
            0xfe, 0x13, 0x09, 0x02, 0x1a, 0x04, 0xc0, 0x2e, 0x09, 0x55, 0x2e, 0x44, 0x3f, 0x80,
            0x00, 0x00, 0x2f, 0x2f, 0xff, 0x13,
        ],
    );
}
