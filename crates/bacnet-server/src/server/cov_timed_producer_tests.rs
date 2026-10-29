//! Producers outside WriteProperty and `write_local` capture timestamped
//! COV-multiple changes at commit (#856, part 2): every attempt of a
//! WritePropertyMultiple, Staging target writes, and the exact Life Safety
//! paths.
//!
//! The transport moves the Device clock when it sends the service response,
//! which the server always does after the mutation and before its COV fanout,
//! so a change stamped at preparation shows the later time. Producers that
//! run before the response are held behind DISABLE_INITIATION instead and
//! released by a later change at a later time.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{EngineeringUnits, LifeSafetyOperation};
use bacnet_types::primitives::Time;

type Row = (PropertyIdentifier, Vec<u8>, Option<Time>);

fn encode(value: PropertyValue) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &value).unwrap();
    encoded.to_vec()
}

/// Encoded Status_Flags with `bits` in the high nibble (IN_ALARM = 0x8,
/// FAULT = 0x4, OVERRIDDEN = 0x2, OUT_OF_SERVICE = 0x1).
fn flags(bits: u8) -> Vec<u8> {
    encode(PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits << 4],
    })
}

/// Rows of one monitored object, in wire order.
fn object_rows(notification: &COVNotificationMultipleRequest, oid: ObjectIdentifier) -> Vec<Row> {
    notification
        .list_of_cov_notifications
        .iter()
        .filter(|item| item.monitored_object_identifier == oid)
        .flat_map(|item| &item.list_of_values)
        .map(|value| {
            (
                value.property_identifier,
                value.value.clone(),
                value.time_of_change,
            )
        })
        .collect()
}

fn property_rows(rows: &[Row], property: PropertyIdentifier) -> Vec<(Vec<u8>, Option<Time>)> {
    rows.iter()
        .filter(|(p, _, _)| *p == property)
        .map(|(_, value, time)| (value.clone(), *time))
        .collect()
}

/// One WritePropertyMultiple carrying `writes` as separate attempts, in order.
/// The clock moves to `after_response` when the response is sent.
async fn write_multiple(
    h: &mut Harness,
    writes: &[(
        ObjectIdentifier,
        PropertyIdentifier,
        PropertyValue,
        Option<u8>,
    )],
    after_response: u8,
) {
    *h.after_ack.lock().unwrap() = Some(at(after_response));
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: writes
            .iter()
            .map(
                |(object, property, value, priority)| WriteAccessSpecification {
                    object_identifier: *object,
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: *property,
                        property_array_index: None,
                        value: encode(value.clone()),
                        priority: *priority,
                    }],
                },
            )
            .collect(),
    };
    let mut body = BytesMut::new();
    request.encode(&mut body).unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
        .await;
}

fn pv_write(
    value: f32,
) -> (
    ObjectIdentifier,
    PropertyIdentifier,
    PropertyValue,
    Option<u8>,
) {
    (av1(), PV, PropertyValue::Real(value), Some(8))
}

#[tokio::test(start_paused = true)]
async fn wpm_reports_every_attempt_at_its_commit_time() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe(false).await;
    h.notification().await;
    h.set_clock(10);
    write_multiple(
        &mut h,
        &[pv_write(10.0), pv_write(20.0), pv_write(30.0)],
        20,
    )
    .await;
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![
            (real(10.0), Some(time(10))),
            (real(20.0), Some(time(10))),
            (real(30.0), Some(time(10))),
        ],
        "each attempt is its own change, stamped when it committed"
    );
    assert_eq!(envelope(&report), Some((at(10).local_date, time(10))));
    assert_eq!(*h.clock.0.lock().unwrap(), at(20), "prepared later");
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn wpm_a_b_a_within_one_request_is_conveyed() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe(false).await;
    h.notification().await;
    // From the initial 0.0, out and back within one request: the final value
    // equals the last one conveyed, yet three changes happened.
    h.set_clock(11);
    write_multiple(&mut h, &[pv_write(10.0), pv_write(20.0), pv_write(0.0)], 21).await;
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![
            (real(10.0), Some(time(11))),
            (real(20.0), Some(time(11))),
            (real(0.0), Some(time(11))),
        ]
    );
    // A value repeated later in the same clock tick is a distinct change.
    h.set_clock(12);
    write_multiple(&mut h, &[pv_write(5.0), pv_write(6.0), pv_write(5.0)], 22).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![
            (real(5.0), Some(time(12))),
            (real(6.0), Some(time(12))),
            (real(5.0), Some(time(12))),
        ]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn wpm_committed_prefix_before_a_failed_attempt_keeps_its_commit_time() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe(false).await;
    h.notification().await;
    h.set_clock(13);
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap();
    write_multiple(
        &mut h,
        &[
            pv_write(10.0),
            pv_write(20.0),
            (missing, PV, PropertyValue::Real(1.0), Some(8)),
        ],
        23,
    )
    .await;
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![(real(10.0), Some(time(13))), (real(20.0), Some(time(13)))]
    );
    h.server.stop().await.unwrap();
}

fn staging() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 3).unwrap()
}

fn bo9() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 9).unwrap()
}

/// STG-3 drives BO-9, which is missing at startup, so the startup plan fails
/// and the stage is applied by the first write after BO-9 exists.
fn faulted_staging(db: &mut ObjectDatabase) {
    let stage = |values: Vec<bool>, limit: f32| BACnetStageLimitValue {
        limit,
        values,
        deadband: 1.0,
    };
    db.add(Box::new(
        StagingObject::new(
            3,
            "STG-3",
            StagingConfig {
                present_value: 5.0,
                min_present_value: 0.0,
                units: EngineeringUnits::DEGREES_CELSIUS,
                priority_for_writing: 8,
                stages: vec![stage(vec![false], 10.0), stage(vec![true], 20.0)],
                target_references: vec![BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: bo9(),
                }],
                stage_names: None,
            },
        )
        .unwrap(),
    ))
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn staging_target_write_carries_its_commit_time() {
    let mut h = Harness::start_with(ServerConfig::default(), faulted_staging).await;
    h.server
        .database()
        .write()
        .await
        .add(Box::new(BinaryOutputObject::new(9, "BO-9").unwrap()))
        .unwrap();
    h.subscribe_specs(
        false,
        vec![(bo9(), vec![(PV, true)]), (av1(), vec![(PV, true)])],
    )
    .await;
    h.notification().await;
    // The Staging write commits BO-9 under the plan's own guard. Nothing
    // subscribes to the Staging object itself here, so its completion has no
    // subscriber.
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(31);
    h.server
        .write_local(
            &staging(),
            PV,
            None,
            PropertyValue::Real(15.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    h.no_notification().await;
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(32);
    h.write_local(1.0).await;
    let report = h.notification().await;
    assert_eq!(
        property_rows(&object_rows(&report, bo9()), PV),
        vec![(encode(PropertyValue::Enumerated(1)), Some(time(31)))],
        "the target write is captured when it committed: {report:?}"
    );
    assert_eq!(
        property_rows(&object_rows(&report, av1()), PV),
        vec![(real(1.0), Some(time(32)))]
    );
    h.server.stop().await.unwrap();
}

fn point() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

fn life_safety_point(db: &mut ObjectDatabase) {
    db.add(Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()))
        .unwrap();
}

fn out_of_service(
    value: bool,
) -> (
    ObjectIdentifier,
    PropertyIdentifier,
    PropertyValue,
    Option<u8>,
) {
    (
        point(),
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(value),
        None,
    )
}

#[tokio::test(start_paused = true)]
async fn life_safety_write_property_change_reports_its_commit_time() {
    let mut h = Harness::start_with(ServerConfig::default(), life_safety_point).await;
    h.subscribe_specs(false, vec![(point(), vec![(PV, true)])])
        .await;
    h.notification().await;
    h.set_clock(40);
    *h.after_ack.lock().unwrap() = Some(at(50));
    let (object, property, value, priority) = out_of_service(true);
    let mut body = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: encode(value),
        priority,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    let report = h.notification().await;
    assert_eq!(
        property_rows(&object_rows(&report, point()), SF),
        vec![(flags(0x1), Some(time(40)))],
        "{report:?}"
    );
    assert_eq!(envelope(&report), Some((at(40).local_date, time(40))));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn life_safety_wpm_reports_each_exact_change_of_an_a_b_a_request() {
    let mut h = Harness::start_with(ServerConfig::default(), life_safety_point).await;
    h.subscribe_specs(false, vec![(point(), vec![(PV, true)])])
        .await;
    h.notification().await;
    h.set_clock(41);
    write_multiple(&mut h, &[out_of_service(true), out_of_service(false)], 51).await;
    let report = h.notification().await;
    assert_eq!(
        property_rows(&object_rows(&report, point()), SF),
        vec![(flags(0x1), Some(time(41))), (flags(0x0), Some(time(41)))],
        "both Out_Of_Service transitions are conveyed: {report:?}"
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn life_safety_operation_change_reports_its_commit_time() {
    let mut h = Harness::start_with(
        ServerConfig {
            policy: Some(Arc::new(
                crate::FnPolicy::new().life_safety_operation(|_| true),
            )),
            ..ServerConfig::default()
        },
        life_safety_point,
    )
    .await;
    h.server
        .set_life_safety_operation_expected_local(&point(), LifeSafetyOperation::SILENCE)
        .await
        .unwrap();
    h.subscribe_specs(
        false,
        vec![(point(), vec![(PropertyIdentifier::SILENCED, true)])],
    )
    .await;
    h.notification().await;
    h.set_clock(42);
    *h.after_ack.lock().unwrap() = Some(at(52));
    let mut body = BytesMut::new();
    LifeSafetyOperationRequest {
        requesting_process_identifier: 9,
        requesting_source: "operator".into(),
        request: LifeSafetyOperation::SILENCE,
        object_identifier: Some(point()),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, body)
        .await;
    let report = h.notification().await;
    let silenced = property_rows(&object_rows(&report, point()), PropertyIdentifier::SILENCED);
    assert_eq!(silenced.len(), 1, "{report:?}");
    assert_eq!(silenced[0].1, Some(time(42)), "{report:?}");
    assert_eq!(envelope(&report), Some((at(42).local_date, time(42))));
    h.server.stop().await.unwrap();
}

/// A Life Safety Point whose Present_Value a write can set, unlike the bundled
/// one, so one request can change it alongside Out_Of_Service.
struct WritablePoint {
    present_value: u32,
    out_of_service: bool,
}

fn writable_point() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap()
}

impl bacnet_objects::traits::BACnetObject for WritablePoint {
    fn object_identifier(&self) -> ObjectIdentifier {
        writable_point()
    }

    fn object_name(&self) -> &str {
        "LSP-2"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => {
                PropertyValue::ObjectIdentifier(writable_point())
            }
            PropertyIdentifier::OBJECT_NAME => PropertyValue::CharacterString("LSP-2".into()),
            PropertyIdentifier::OBJECT_TYPE => {
                PropertyValue::Enumerated(ObjectType::LIFE_SAFETY_POINT.to_raw())
            }
            PropertyIdentifier::PRESENT_VALUE => PropertyValue::Enumerated(self.present_value),
            PropertyIdentifier::STATUS_FLAGS => PropertyValue::BitString {
                unused_bits: 4,
                data: vec![if self.out_of_service { 0x10 } else { 0 }],
            },
            PropertyIdentifier::OUT_OF_SERVICE => PropertyValue::Boolean(self.out_of_service),
            _ => {
                return Err(Error::protocol(
                    bacnet_types::enums::ErrorClass::PROPERTY,
                    bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
                ));
            }
        })
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        match (property, value) {
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Enumerated(value)) => {
                self.present_value = value;
            }
            (PropertyIdentifier::OUT_OF_SERVICE, PropertyValue::Boolean(value)) => {
                self.out_of_service = value;
            }
            _ => {
                return Err(Error::protocol(
                    bacnet_types::enums::ErrorClass::PROPERTY,
                    bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED,
                ));
            }
        }
        Ok(())
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
        ])
    }
}

#[tokio::test(start_paused = true)]
async fn life_safety_wpm_conveys_a_reference_its_exact_fanout_did_not_select() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(WritablePoint {
            present_value: 0,
            out_of_service: false,
        }))
        .unwrap();
    })
    .await;
    // Two contexts: Status_Flags in one, Present_Value in the other.
    h.subscribe_process(
        856,
        false,
        vec![(writable_point(), vec![(SF, true)])],
        Some(10),
    )
    .await;
    h.notification().await;
    h.subscribe_process(
        857,
        false,
        vec![(writable_point(), vec![(PV, true)])],
        Some(10),
    )
    .await;
    h.notification().await;
    h.set_clock(43);
    // Present_Value changes for the request as a whole; Status_Flags goes out
    // and back, so the request's exact fanout selects only the PV reference.
    let start = tokio::time::Instant::now();
    write_multiple(
        &mut h,
        &[
            (
                writable_point(),
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
                None,
            ),
            (writable_point(), PV, PropertyValue::Enumerated(3), None),
            (
                writable_point(),
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(false),
                None,
            ),
        ],
        53,
    )
    .await;
    let first = h.notification().await;
    let second = h.notification().await;
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "both contexts are told at once, not at the 10 s deadline"
    );
    let (flags_context, pv_context) = if first.subscriber_process_identifier == 856 {
        (&first, &second)
    } else {
        (&second, &first)
    };
    assert_eq!(pv_context.subscriber_process_identifier, 857);
    assert_eq!(
        property_rows(&object_rows(pv_context, writable_point()), PV)
            .last()
            .cloned(),
        Some((encode(PropertyValue::Enumerated(3)), Some(time(43))))
    );
    assert_eq!(
        property_rows(&object_rows(flags_context, writable_point()), SF),
        vec![(flags(0x1), Some(time(43))), (flags(0x0), Some(time(43)))],
        "each attempt's Status_Flags change, at its commit: {flags_context:?}"
    );
    h.server.stop().await.unwrap();
}
