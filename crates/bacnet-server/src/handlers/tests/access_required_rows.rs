//! The Table 12-40 and Table 12-30 required rows over ReadProperty and
//! WriteProperty (#1073): the credential's derived status and disable
//! reasons, its validity window against the database clock, its two
//! constructed arrays, and the door's pulse times and command priority.

use bacnet_types::error::RemoteError;
use std::sync::Arc;

use super::*;
use PropertyIdentifier as P;
use bacnet_objects::access_control::{AccessCredentialObject, AccessDoorObject};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_types::constructed::{
    BACnetAssignedAccessRights, BACnetAuthenticationFactor, BACnetCredentialAuthenticationFactor,
    BACnetDeviceObjectReference,
};
use bacnet_types::enums::{AccessAuthenticationFactorDisable, AuthenticationFactorType};
use bacnet_types::primitives::{Date, Time};

const EMPTY: &[u8] = &[];

fn db_with(object: Box<dyn BACnetObject>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    (db, oid)
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    let mut property_value = BytesMut::new();
    encode_property_value(&mut property_value, &value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: property_value.to_vec(),
        priority,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

/// The raw propertyValue a ReadProperty ACK carries.
fn read_wire(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response)?;
    Ok(ReadPropertyACK::decode(&response).unwrap().property_value)
}

fn assert_property_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY/{expected:?}, got {result:?}"
    );
}

struct FixedClock(ClockFrame);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
    }
}

fn date(year: u16, month: u8, day: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week: Date::UNSPECIFIED,
    }
}

fn midnight() -> Time {
    Time {
        hour: 0,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

fn date_time(date: Date, time: Time) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

#[test]
fn wp_rp_access_credential_disable_drives_status_and_reasons() {
    let (mut db, oid) = db_with(Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()));
    assert_eq!(
        read_wire(&db, oid, P::CREDENTIAL_STATUS, None).unwrap(),
        [0x91, 1]
    );
    assert_eq!(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        EMPTY
    );

    // DISABLE_MANUAL adds DISABLED_MANUAL (9) and the status goes INACTIVE.
    write(
        &mut db,
        oid,
        P::CREDENTIAL_DISABLE,
        None,
        PropertyValue::Enumerated(2),
        None,
    )
    .unwrap();
    let disabled = [
        read_wire(&db, oid, P::CREDENTIAL_STATUS, None).unwrap(),
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        read_wire(&db, oid, P::CREDENTIAL_DISABLE, None).unwrap(),
    ];
    assert_eq!(disabled, [vec![0x91, 0], vec![0x91, 9], vec![0x91, 2]]);
    for raw in [4, 63, 65536] {
        assert_property_error(
            write(
                &mut db,
                oid,
                P::CREDENTIAL_DISABLE,
                None,
                PropertyValue::Enumerated(raw),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_property_error(
        write(
            &mut db,
            oid,
            P::REASON_FOR_DISABLE,
            None,
            PropertyValue::Enumerated(0),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_property_error(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, Some(1)),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_eq!(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        [0x91, 9]
    );

    // Back to NONE drops the reason the previous value added.
    write(
        &mut db,
        oid,
        P::CREDENTIAL_DISABLE,
        None,
        PropertyValue::Enumerated(0),
        None,
    )
    .unwrap();
    assert_eq!(
        read_wire(&db, oid, P::CREDENTIAL_STATUS, None).unwrap(),
        [0x91, 1]
    );
    assert_eq!(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        EMPTY
    );
}

#[test]
fn wp_rp_access_credential_window_follows_the_database_clock() {
    let (mut db, oid) = db_with(Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()));
    db.set_clock_reader(Some(Arc::new(FixedClock(ClockFrame {
        local_date: date(2026, 3, 15),
        local_time: Time {
            hour: 12,
            ..midnight()
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }))));
    let open = || {
        date_time(
            Date {
                year: 0xFF,
                month: 0xFF,
                day: 0xFF,
                day_of_week: 0xFF,
            },
            Time {
                hour: 0xFF,
                minute: 0xFF,
                second: 0xFF,
                hundredths: 0xFF,
            },
        )
    };

    // Activation on 1 April 2026: not yet active on 15 March.
    write(
        &mut db,
        oid,
        P::ACTIVATION_TIME,
        None,
        date_time(date(2026, 4, 1), midnight()),
        None,
    )
    .unwrap();
    assert_eq!(
        read_wire(&db, oid, P::ACTIVATION_TIME, None).unwrap(),
        [0xA4, 126, 4, 1, 0xFF, 0xB4, 0, 0, 0, 0]
    );
    assert_eq!(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        [0x91, 3]
    );
    assert_eq!(
        read_wire(&db, oid, P::CREDENTIAL_STATUS, None).unwrap(),
        [0x91, 0]
    );

    // Open start, expiry on 1 March 2026: expired on 15 March.
    write(&mut db, oid, P::ACTIVATION_TIME, None, open(), None).unwrap();
    write(
        &mut db,
        oid,
        P::EXPIRATION_TIME,
        None,
        date_time(date(2026, 3, 1), midnight()),
        None,
    )
    .unwrap();
    assert_eq!(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        [0x91, 4]
    );
    write(&mut db, oid, P::EXPIRATION_TIME, None, open(), None).unwrap();
    assert_eq!(
        read_wire(&db, oid, P::REASON_FOR_DISABLE, None).unwrap(),
        EMPTY
    );
    assert_eq!(
        read_wire(&db, oid, P::CREDENTIAL_STATUS, None).unwrap(),
        [0x91, 1]
    );

    // A partly specified moment and a lone Date are refused.
    let partial = date_time(
        Date {
            year: 0xFF,
            ..date(2026, 4, 1)
        },
        midnight(),
    );
    assert_property_error(
        write(&mut db, oid, P::ACTIVATION_TIME, None, partial, None),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_property_error(
        write(
            &mut db,
            oid,
            P::EXPIRATION_TIME,
            None,
            PropertyValue::Date(date(2026, 4, 1)),
            None,
        ),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(
        read_wire(&db, oid, P::ACTIVATION_TIME, None).unwrap(),
        [0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF]
    );
}

#[test]
fn rp_wp_access_credential_arrays_and_global_identifier_on_the_wire() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .set_authentication_factors(vec![BACnetCredentialAuthenticationFactor {
            disable: AccessAuthenticationFactorDisable::NONE,
            authentication_factor: BACnetAuthenticationFactor {
                format_type: AuthenticationFactorType::WIEGAND26,
                format_class: 0,
                value: vec![0x12, 0x34, 0x56],
            },
        }])
        .unwrap();
    let rights = |instance, enable| BACnetAssignedAccessRights {
        assigned_access_rights: BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, instance).unwrap(),
        },
        enable,
    };
    credential
        .set_assigned_access_rights(vec![rights(5, true), rights(6, false)])
        .unwrap();
    let (mut db, oid) = db_with(Box::new(credential));

    let factor: &[u8] = &[
        0x09, 0x00, 0x1E, 0x09, 0x08, 0x19, 0x00, 0x2B, 0x12, 0x34, 0x56, 0x1F,
    ];
    assert_eq!(
        read_wire(&db, oid, P::AUTHENTICATION_FACTORS, None).unwrap(),
        factor
    );
    assert_eq!(
        read_wire(&db, oid, P::AUTHENTICATION_FACTORS, Some(1)).unwrap(),
        factor
    );
    assert_eq!(
        read_wire(&db, oid, P::AUTHENTICATION_FACTORS, Some(0)).unwrap(),
        [0x21, 1]
    );
    // Access Rights 5 and 6 are (34 << 22) | 5 and | 6.
    let first: &[u8] = &[0x0E, 0x1C, 0x08, 0x80, 0x00, 0x05, 0x0F, 0x19, 0x01];
    let second: &[u8] = &[0x0E, 0x1C, 0x08, 0x80, 0x00, 0x06, 0x0F, 0x19, 0x00];
    assert_eq!(
        read_wire(&db, oid, P::ASSIGNED_ACCESS_RIGHTS, None).unwrap(),
        [first, second].concat()
    );
    assert_eq!(
        read_wire(&db, oid, P::ASSIGNED_ACCESS_RIGHTS, Some(2)).unwrap(),
        second
    );
    assert_eq!(
        read_wire(&db, oid, P::ASSIGNED_ACCESS_RIGHTS, Some(0)).unwrap(),
        [0x21, 2]
    );
    for (property, past_end) in [
        (P::AUTHENTICATION_FACTORS, 2),
        (P::ASSIGNED_ACCESS_RIGHTS, 3),
    ] {
        assert_property_error(
            read_wire(&db, oid, property, Some(past_end)),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
        // Table 12-40 makes both R: no write reaches them.
        for index in [None, Some(0), Some(1)] {
            assert_property_error(
                write(
                    &mut db,
                    oid,
                    property,
                    index,
                    PropertyValue::Unsigned(0),
                    None,
                ),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
    }
    assert_eq!(
        read_wire(&db, oid, P::AUTHENTICATION_FACTORS, None).unwrap(),
        factor
    );

    // Global_Identifier is the table's W row, an Unsigned32.
    write(
        &mut db,
        oid,
        P::GLOBAL_IDENTIFIER,
        None,
        PropertyValue::Unsigned(0xFFFF_FFFF),
        None,
    )
    .unwrap();
    assert_eq!(
        read_wire(&db, oid, P::GLOBAL_IDENTIFIER, None).unwrap(),
        [0x24, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    assert_property_error(
        write(
            &mut db,
            oid,
            P::GLOBAL_IDENTIFIER,
            None,
            PropertyValue::Unsigned(1 << 32),
            None,
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}

#[test]
fn wp_rp_access_door_times_and_command_priority() {
    let (mut db, oid) = db_with(Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()));
    for property in [
        P::DOOR_PULSE_TIME,
        P::DOOR_EXTENDED_PULSE_TIME,
        P::DOOR_OPEN_TOO_LONG_TIME,
    ] {
        write(
            &mut db,
            oid,
            property,
            None,
            PropertyValue::Unsigned(25),
            None,
        )
        .unwrap();
        assert_eq!(read_wire(&db, oid, property, None).unwrap(), [0x21, 25]);
        assert_property_error(
            write(
                &mut db,
                oid,
                property,
                None,
                PropertyValue::Unsigned(1 << 32),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_property_error(
            write(&mut db, oid, property, None, PropertyValue::Real(2.5), None),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(read_wire(&db, oid, property, None).unwrap(), [0x21, 25]);
    }

    // Current_Command_Priority is NULL on the default and names the slot
    // Present_Value comes from otherwise.
    let ccp = P::CURRENT_COMMAND_PRIORITY;
    assert_eq!(read_wire(&db, oid, ccp, None).unwrap(), [0x00]);
    write(
        &mut db,
        oid,
        P::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        Some(8),
    )
    .unwrap();
    assert_eq!(read_wire(&db, oid, ccp, None).unwrap(), [0x21, 8]);
    assert_property_error(
        write(&mut db, oid, ccp, None, PropertyValue::Unsigned(3), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    write(
        &mut db,
        oid,
        P::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(8),
    )
    .unwrap();
    assert_eq!(read_wire(&db, oid, ccp, None).unwrap(), [0x00]);
}
