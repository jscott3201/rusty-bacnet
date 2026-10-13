//! SubscribeCOV reports for the Access Door, Access Point, Credential Data
//! Input and Load Control rows of the COV criteria table (Clause 13.1,
//! Table 13-1) over the wire (#1061).
//!
//! Each report carries the row's values in the row's order. A change of a
//! trigger sends a report on its own; a value that only rides along waits for
//! the next report. Access Point has no Present_Value, so its report starts
//! with Access_Event. Update_Time and the Access Point event rows have no
//! network write route, and Door_Alarm_State has one only while the door is out
//! of service (#1131), so those tests report them through the application's
//! routes, `report_access_event_local`, `report_credential_read_local` and
//! `report_door_state_local` (#1132). The door's simulation test writes it
//! over the wire instead, as the Load Control test does Requested_Shed_Level.
//! A Load Control's Actual_Shed_Level has no such route, so its test puts an
//! object holding the changed value into the database (`ObjectDatabase::add`
//! replaces by identifier) and runs the fanout a write commit would.
//!
//! The BACnetTimeStamp, BACnetAuthenticationFactor and BACnetShedLevel values
//! go out in their Clause 21 forms (#1133). A Credential Data Input's
//! simulated Present_Value and Reliability are written over the wire while it
//! is out of service (#1168), and so is the Access Point Out_Of_Service whose
//! edges record access events (#1248).
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::access_control::{
    AccessDoorObject, AccessEventReport, AccessPointObject, CredentialDataInputObject,
    CredentialReadReport, DoorStateReport,
};
use bacnet_objects::load_control::LoadControlObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{
    BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat, BACnetShedLevel,
};
use bacnet_types::enums::{
    AccessEvent, AuthenticationFactorType, DoorAlarmState, DoorStatus, DoorValue, LockStatus,
    ObjectType, Reliability,
};
use bacnet_types::primitives::BACnetTimeStamp;

type Values = Vec<(PropertyIdentifier, Vec<u8>)>;

fn encode(value: PropertyValue) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &value).unwrap();
    encoded.to_vec()
}

fn enumerated(value: u32) -> Vec<u8> {
    encode(PropertyValue::Enumerated(value))
}

fn unsigned(value: u64) -> Vec<u8> {
    encode(PropertyValue::Unsigned(value))
}

/// Status_Flags with every flag clear.
fn normal() -> Vec<u8> {
    vec![0x82, 0x04, 0x00]
}

/// Today at `second` past 15:00, as a BACnetTimeStamp.
fn stamp(second: u8) -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: at(0).local_date,
        time: time(second),
    }
}

/// The datetime [2] choice framed around today's Date and the Time at
/// `second` past 15:00, written out by hand.
fn stamp_bytes(second: u8) -> Vec<u8> {
    vec![0x2E, 0xA4, 126, 9, 29, 2, 0xB4, 15, 0, second, 0, 0x2F]
}

/// `(property, value bytes)` of a notification for `oid`, in wire order.
fn values(notification: &COVNotificationRequest, oid: ObjectIdentifier) -> Values {
    assert_eq!(notification.monitored_object_identifier, oid);
    notification
        .list_of_values
        .iter()
        .map(|value| {
            assert_eq!(value.property_array_index, None);
            assert_eq!(value.priority, None);
            (value.property_identifier, value.value.clone())
        })
        .collect()
}

/// SubscribeCOV for `oid` and the answer to it.
async fn subscribe(h: &mut Harness, oid: ObjectIdentifier) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1061,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    response(h).await
}

/// Subscribe to `oid` and return the values of the first report.
async fn subscribed(h: &mut Harness, oid: ObjectIdentifier) -> Values {
    assert_eq!(subscribe(h, oid).await, Ok(()), "SubscribeCOV on {oid:?}");
    values(&h.cov_notification().await, oid)
}

/// WriteProperty of `property` and the answer to it.
async fn try_write(
    h: &mut Harness,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    response(h).await
}

async fn write(
    h: &mut Harness,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) {
    assert_eq!(
        try_write(h, oid, property, value).await,
        Ok(()),
        "write of {property:?}"
    );
}

/// One WritePropertyMultiple of `writes` to `oid`, answered with a SimpleACK.
async fn write_multiple(
    h: &mut Harness,
    oid: ObjectIdentifier,
    writes: Vec<(PropertyIdentifier, Vec<u8>)>,
) {
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .into_iter()
                .map(|(property_identifier, value)| BACnetPropertyValue {
                    property_identifier,
                    property_array_index: None,
                    value,
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
        .await;
    assert_eq!(response(h).await, Ok(()), "WritePropertyMultiple");
}

/// `event` in transaction `tag`, from Access Credential `tag` at `second`
/// past 15:00.
fn access_event(event: AccessEvent, tag: u64, second: u8) -> AccessEventReport {
    let card = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, tag as u32).unwrap();
    AccessEventReport {
        time: Some(stamp(second)),
        credential: Some(card.into()),
        ..AccessEventReport::new(event, tag)
    }
}

/// AP-1, whose last event is `access_event(event, tag, second)`.
fn point(event: AccessEvent, tag: u64, second: u8) -> Box<dyn BACnetObject> {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_access_event(access_event(event, tag, second))
        .unwrap();
    Box::new(point)
}

/// The UNDEFINED authentication factor: format type [0] 0, class [1] 0 and
/// no value [2].
fn undefined_factor_bytes() -> Vec<u8> {
    vec![0x09, 0x00, 0x19, 0x00, 0x28]
}

/// The Wiegand 26 card 12 34 56, class 0.
fn card() -> BACnetAuthenticationFactor {
    BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::WIEGAND26,
        format_class: 0,
        value: vec![0x12, 0x34, 0x56],
    }
}

/// [`card`] as Clause 21 puts it on the wire.
fn card_bytes() -> Vec<u8> {
    vec![0x09, 0x08, 0x19, 0x00, 0x2B, 0x12, 0x34, 0x56]
}

/// Access_Event_Credential naming Access Credential `instance` (below 256),
/// the object under [1].
fn credential_bytes(instance: u8) -> Vec<u8> {
    vec![0x1C, 0x08, 0x00, 0x00, instance]
}

/// The no-credential Access_Event_Credential: Access Credential 4194303.
fn no_credential_bytes() -> Vec<u8> {
    vec![0x1C, 0x08, 0x3F, 0xFF, 0xFF]
}

#[tokio::test(start_paused = true)]
async fn access_point_cov_leads_with_access_event_and_triggers_on_its_time() {
    let oid = ObjectIdentifier::new(ObjectType::ACCESS_POINT, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
    })
    .await;
    // Access_Event_Credential and Access_Event_Authentication_Factor follow
    // the time (#1284, #1132).
    let report = |event: AccessEvent, tag: u64, second: u8, factor: Vec<u8>| {
        vec![
            (PropertyIdentifier::ACCESS_EVENT, enumerated(event.to_raw())),
            (SF, normal()),
            (PropertyIdentifier::ACCESS_EVENT_TAG, unsigned(tag)),
            (PropertyIdentifier::ACCESS_EVENT_TIME, stamp_bytes(second)),
            (
                PropertyIdentifier::ACCESS_EVENT_CREDENTIAL,
                credential_bytes(tag as u8),
            ),
            (
                PropertyIdentifier::ACCESS_EVENT_AUTHENTICATION_FACTOR,
                factor,
            ),
        ]
    };
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(AccessEvent::GRANTED, 1, 7, undefined_factor_bytes())
    );

    // Access_Event, Access_Event_Tag, Access_Event_Credential and the factor
    // only ride along.
    h.server
        .report_access_event_local(&oid, access_event(AccessEvent::DENIED_DENY_ALL, 2, 7))
        .await
        .unwrap();
    h.no_notification().await;

    // A new Access_Event_Time sends a report with the current values.
    let granted = AccessEventReport {
        authentication_factor: Some(card()),
        ..access_event(AccessEvent::GRANTED, 3, 9)
    };
    h.server
        .report_access_event_local(&oid, granted)
        .await
        .unwrap();
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(AccessEvent::GRANTED, 3, 9, card_bytes())
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn access_point_out_of_service_edges_record_events_and_report() {
    const OUT_OF_SERVICE: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
    let oid = ObjectIdentifier::new(ObjectType::ACCESS_POINT, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
    })
    .await;
    // An edge stores the no-credential reference (#1284); the GRANTED event
    // before them came from Access Credential 1.
    let report = |event: AccessEvent, flags: u8, tag: u64, second: u8| {
        let credential = if event == AccessEvent::GRANTED {
            credential_bytes(1)
        } else {
            no_credential_bytes()
        };
        vec![
            (PropertyIdentifier::ACCESS_EVENT, enumerated(event.to_raw())),
            (SF, vec![0x82, 0x04, flags]),
            (PropertyIdentifier::ACCESS_EVENT_TAG, unsigned(tag)),
            (PropertyIdentifier::ACCESS_EVENT_TIME, stamp_bytes(second)),
            (PropertyIdentifier::ACCESS_EVENT_CREDENTIAL, credential),
            (
                PropertyIdentifier::ACCESS_EVENT_AUTHENTICATION_FACTOR,
                undefined_factor_bytes(),
            ),
        ]
    };
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(AccessEvent::GRANTED, 0x00, 1, 7)
    );

    // Entering out of service records OUT_OF_SERVICE as a new transaction at
    // the Device clock's time; one report carries it with the flag (#1248).
    h.set_clock(20);
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(AccessEvent::OUT_OF_SERVICE, 0x10, 2, 20)
    );
    h.no_notification().await;

    // TRUE again is no edge: nothing is recorded and nothing reports.
    h.set_clock(25);
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    h.no_notification().await;

    // The return to service records OUT_OF_SERVICE_RELINQUISHED.
    h.set_clock(30);
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(AccessEvent::OUT_OF_SERVICE_RELINQUISHED, 0x00, 3, 30)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn access_point_out_of_service_round_trip_reports_without_a_clock() {
    const OUT_OF_SERVICE: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
    let oid = ObjectIdentifier::new(ObjectType::ACCESS_POINT, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(AccessPointObject::new(1, "AP-1").unwrap()))
            .unwrap();
    })
    .await;
    // Without a usable Device clock an edge stamps its new tag as a sequence
    // number [1]. Before any event the time is the unspecified date and time.
    h.server.database().write().await.set_clock_reader(None);
    let unspecified = vec![
        0x2E, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF, 0x2F,
    ];
    // No event here carries a credential.
    let report = |event: AccessEvent, tag: u64, time: Vec<u8>| {
        vec![
            (PropertyIdentifier::ACCESS_EVENT, enumerated(event.to_raw())),
            (SF, normal()),
            (PropertyIdentifier::ACCESS_EVENT_TAG, unsigned(tag)),
            (PropertyIdentifier::ACCESS_EVENT_TIME, time),
            (
                PropertyIdentifier::ACCESS_EVENT_CREDENTIAL,
                no_credential_bytes(),
            ),
            (
                PropertyIdentifier::ACCESS_EVENT_AUTHENTICATION_FACTOR,
                undefined_factor_bytes(),
            ),
        ]
    };
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(AccessEvent::NONE, 0, unspecified)
    );

    // One WritePropertyMultiple takes the point out of service and back. Both
    // edges are recorded, so the tag moves on by two and Access_Event ends at
    // OUT_OF_SERVICE_RELINQUISHED. Status_Flags ends where it started, but
    // Access_Event_Time, the Table 13-1 trigger, moved to sequence number 2,
    // so the report goes out.
    write_multiple(
        &mut h,
        oid,
        vec![
            (OUT_OF_SERVICE, encode(PropertyValue::Boolean(true))),
            (OUT_OF_SERVICE, encode(PropertyValue::Boolean(false))),
        ],
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(
            AccessEvent::OUT_OF_SERVICE_RELINQUISHED,
            2,
            vec![0x19, 0x02]
        )
    );
    // The round trip back out of service and in again moves on to 4.
    write_multiple(
        &mut h,
        oid,
        vec![
            (OUT_OF_SERVICE, encode(PropertyValue::Boolean(true))),
            (OUT_OF_SERVICE, encode(PropertyValue::Boolean(false))),
        ],
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(
            AccessEvent::OUT_OF_SERVICE_RELINQUISHED,
            4,
            vec![0x19, 0x04]
        )
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// CDI-1, a reader of Wiegand 26 cards (class 0), having read [`card`] at
/// `second` past 15:00.
fn reader(second: u8) -> Box<dyn BACnetObject> {
    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    reader
        .set_supported_formats([(
            BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::WIEGAND26),
            0,
        )])
        .unwrap();
    reader.set_present_value(card(), stamp(second)).unwrap();
    Box::new(reader)
}

#[tokio::test(start_paused = true)]
async fn credential_data_input_cov_reports_and_triggers_on_update_time() {
    let oid = ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(reader(7)).unwrap();
    })
    .await;
    let report = |second: u8| {
        vec![
            // format type [0] WIEGAND26, format class [1] 0, value [2].
            (PV, card_bytes()),
            (SF, normal()),
            (PropertyIdentifier::UPDATE_TIME, stamp_bytes(second)),
        ]
    };
    assert_eq!(subscribed(&mut h, oid).await, report(7));

    // The same input read again moves Update_Time alone, and that reports.
    let read_at = |second| CredentialReadReport {
        update_time: Some(stamp(second)),
        ..CredentialReadReport::new(card())
    };
    h.server
        .report_credential_read_local(&oid, read_at(9))
        .await
        .unwrap();
    assert_eq!(values(&h.cov_notification().await, oid), report(9));
    h.no_notification().await;
    // A read stamped with the time Update_Time already holds moves nothing.
    h.server
        .report_credential_read_local(&oid, read_at(9))
        .await
        .unwrap();
    h.no_notification().await;

    // Row values unchanged: a Description write doesn't report.
    write(
        &mut h,
        oid,
        PropertyIdentifier::DESCRIPTION,
        encode(PropertyValue::CharacterString("lobby reader".into())),
    )
    .await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn credential_data_input_simulated_rows_report_and_restore() {
    const OUT_OF_SERVICE: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
    let oid = ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        reader
            .set_supported_formats([(
                BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::WIEGAND26),
                0,
            )])
            .unwrap();
        reader
            .set_present_value(
                BACnetAuthenticationFactor {
                    format_type: AuthenticationFactorType::WIEGAND26,
                    format_class: 0,
                    value: vec![0x12, 0x34, 0x56],
                },
                stamp(7),
            )
            .unwrap();
        db.add(Box::new(reader)).unwrap();
    })
    .await;
    // format type [0] WIEGAND26, format class [1] 0, a three-octet value [2].
    let card = |value: [u8; 3]| [vec![0x09, 0x08, 0x19, 0x00, 0x2B], value.to_vec()].concat();
    let report = |value: [u8; 3], flags: u8, second: u8| {
        vec![
            (PV, card(value)),
            (SF, vec![0x82, 0x04, flags]),
            (PropertyIdentifier::UPDATE_TIME, stamp_bytes(second)),
        ]
    };
    let device = [0x12, 0x34, 0x56];
    let simulated = [0x65, 0x43, 0x21];
    assert_eq!(subscribed(&mut h, oid).await, report(device, 0x00, 7));

    // In service the write is refused and nothing reports.
    assert_eq!(
        try_write(&mut h, oid, PV, card(simulated)).await,
        Err(ErrorCode::WRITE_ACCESS_DENIED)
    );
    h.no_notification().await;

    // The OUT_OF_SERVICE flag reports; then a simulated read, stamped from
    // the Device clock, reports on its own.
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(device, 0x10, 7)
    );
    h.set_clock(20);
    write(&mut h, oid, PV, card(simulated)).await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(simulated, 0x10, 20)
    );
    h.no_notification().await;

    // A simulated fault sets FAULT and reports; Update_Time stays.
    write(
        &mut h,
        oid,
        PropertyIdentifier::RELIABILITY,
        enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(simulated, 0x50, 20)
    );
    h.no_notification().await;

    // Simulating the reader's own card later reports again.
    h.set_clock(25);
    write(&mut h, oid, PV, card(device)).await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(device, 0x50, 25)
    );
    h.no_notification().await;

    // The return to service brings back the reader's read, its time and
    // NO_FAULT_DETECTED, in one report.
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(device, 0x00, 7)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// LC-1 at requested level `requested`, achieving level 1, with the
/// Shed_Duration given.
fn load_control(requested: u64, duration: u64) -> Box<dyn BACnetObject> {
    let mut object = LoadControlObject::new(1, "LC-1").unwrap();
    object
        .set_requested_shed_level(BACnetShedLevel::Level(requested))
        .unwrap();
    object
        .set_actual_shed_level(BACnetShedLevel::Level(1))
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::SHED_DURATION,
            None,
            PropertyValue::Unsigned(duration),
            None,
        )
        .unwrap();
    Box::new(object)
}

#[tokio::test(start_paused = true)]
async fn load_control_cov_reports_and_triggers_on_its_shed_rows() {
    const REQUESTED: PropertyIdentifier = PropertyIdentifier::REQUESTED_SHED_LEVEL;
    let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(LoadControlObject::new(1, "LC-1").unwrap()))
            .unwrap();
    })
    .await;
    // Start_Time is a BACnetDateTime: an application Date and Time.
    let unspecified = vec![0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF];
    // Duty_Window isn't served yet, so the report leaves it out. The
    // requested level goes out as its BACnetShedLevel choice.
    let report = |requested: Vec<u8>, duration: u64| {
        vec![
            (PV, enumerated(0)),
            (SF, normal()),
            (REQUESTED, requested),
            (PropertyIdentifier::START_TIME, unspecified.clone()),
            (PropertyIdentifier::SHED_DURATION, unsigned(duration)),
        ]
    };
    // level [1] 0, the LEVEL choice's default.
    assert_eq!(subscribed(&mut h, oid).await, report(vec![0x19, 0x00], 0));

    // Actual_Shed_Level isn't in the row, so its change doesn't report.
    h.replace_and_fan_out(load_control(0, 0)).await;
    h.no_notification().await;

    // A Shed_Duration write sends a report.
    write(
        &mut h,
        oid,
        PropertyIdentifier::SHED_DURATION,
        unsigned(3600),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(vec![0x19, 0x00], 3600)
    );
    h.no_notification().await;

    // So does a Requested_Shed_Level write, as level [1] 3 and as percent
    // [0] 80.
    for requested in [vec![0x19, 0x03], vec![0x09, 0x50]] {
        write(&mut h, oid, REQUESTED, requested.clone()).await;
        assert_eq!(
            values(&h.cov_notification().await, oid),
            report(requested, 3600)
        );
        h.no_notification().await;
    }

    // A level the application sets reports the same way.
    h.replace_and_fan_out(load_control(5, 3600)).await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(vec![0x19, 0x05], 3600)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// The three shed levels of `oid`, encoded as ReadProperty serves them.
async fn shed_levels(h: &Harness, oid: ObjectIdentifier) -> [Vec<u8>; 3] {
    let db = h.server.database().read().await;
    let object = db.get(&oid).unwrap();
    [
        PropertyIdentifier::REQUESTED_SHED_LEVEL,
        PropertyIdentifier::EXPECTED_SHED_LEVEL,
        PropertyIdentifier::ACTUAL_SHED_LEVEL,
    ]
    .map(|p| encode(object.read_property(p, None).unwrap()))
}

#[tokio::test(start_paused = true)]
async fn load_control_requested_shed_level_write_takes_the_choice_form() {
    const REQUESTED: PropertyIdentifier = PropertyIdentifier::REQUESTED_SHED_LEVEL;
    let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(LoadControlObject::new(1, "LC-1").unwrap()))
            .unwrap();
    })
    .await;
    // amount [2] 12.5 kW. The object stays SHED_INACTIVE, so Expected and
    // Actual take the AMOUNT default, 0.0.
    let amount = vec![0x2C, 0x41, 0x48, 0x00, 0x00];
    let zero_kw = vec![0x2C, 0x00, 0x00, 0x00, 0x00];
    let after = [amount.clone(), zero_kw.clone(), zero_kw];
    write(&mut h, oid, REQUESTED, amount).await;
    assert_eq!(shed_levels(&h, oid).await, after);

    // The application-tagged forms served before #1133, and a context tag
    // the CHOICE lacks, are the wrong datatype; asking for more load than
    // the baseline is out of range. Nothing changes.
    for (value, code) in [
        (unsigned(50), ErrorCode::INVALID_DATA_TYPE),
        (
            encode(PropertyValue::Real(12.5)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (vec![0x39, 0x01], ErrorCode::INVALID_DATA_TYPE),
        // percent [0] 101
        (vec![0x09, 0x65], ErrorCode::VALUE_OUT_OF_RANGE),
        // amount [2] -1.0
        (
            vec![0x2C, 0xBF, 0x80, 0x00, 0x00],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_eq!(try_write(&mut h, oid, REQUESTED, value).await, Err(code));
        assert_eq!(shed_levels(&h, oid).await, after);
    }
    h.server.stop().await.unwrap();
}

#[path = "table_13_1_door_cov_tests.rs"]
mod door;

#[path = "table_13_1_access_input_tests.rs"]
mod access_inputs;
