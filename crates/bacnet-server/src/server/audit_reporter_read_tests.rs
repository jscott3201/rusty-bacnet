use super::*;
use bacnet_objects::{audit::AuditReporterObject, traits::BACnetObject};
use bacnet_services::{
    common::PropertyReference,
    read_property::{ReadPropertyACK, ReadPropertyRequest},
    rpm::{ReadAccessSpecification, ReadPropertyMultipleACK, ReadPropertyMultipleRequest},
};
use bacnet_types::constructed::BACnetObjectSelector as Selector;
use std::{borrow::Cow, sync::atomic::AtomicUsize};

#[path = "audit_reporter_read_boundary_tests.rs"]
mod boundary;

#[path = "audit_reporter_range_file_tests.rs"]
mod range_file;

fn read_reporter() -> AuditReporterObject {
    let mut reporter = reporter();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::READ);
    reporter.set_auditable_operations(operations);
    reporter
}

fn rp(target: ObjectIdentifier, property: PropertyIdentifier, index: Option<u32>) -> Bytes {
    let mut bytes = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: target,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut bytes);
    bytes.freeze()
}

fn spec(
    target: ObjectIdentifier,
    properties: &[(PropertyIdentifier, Option<u32>)],
) -> ReadAccessSpecification {
    ReadAccessSpecification {
        object_identifier: target,
        list_of_property_references: properties
            .iter()
            .map(|&(id, index)| PropertyReference {
                property_identifier: id,
                property_array_index: index,
            })
            .collect(),
    }
}

fn rpm(specs: Vec<ReadAccessSpecification>) -> Bytes {
    let mut bytes = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: specs,
    }
    .encode(&mut bytes);
    bytes.freeze()
}

fn records(fixture: &Fixture) -> Vec<BACnetAuditNotification> {
    notifications(&fixture.transport.sent)
        .into_iter()
        .map(|mut request| {
            assert_eq!(request.notifications.len(), 1);
            request.notifications.remove(0)
        })
        .collect()
}

fn expected(
    target: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    invoke: u8,
    sequence: u16,
    result: Option<(ErrorClass, ErrorCode)>,
) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: Some(BACnetTimeStamp::SequenceNumber(sequence)),
        source_device: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(SOURCE),
        }),
        source_object: None,
        operation: AuditOperation::READ,
        source_comment: None,
        target_comment: None,
        invoke_id: Some(invoke),
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
        target_object: Some(target),
        target_property: Some(AuditPropertyReference {
            property_identifier: property,
            property_array_index: index.map(u64::from),
        }),
        target_priority: None,
        target_value: None,
        current_value: None,
        result,
    }
}

fn wire(response: &Apdu) -> Bytes {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, response).unwrap();
    bytes.freeze()
}

#[tokio::test]
async fn audit_reporter_rp_success_exact_identity_no_values_and_unchanged_response() {
    let mut fixture = server(read_reporter()).await;
    let mut plain = server(read_reporter()).await;
    plain.server.config.audit_reporter = None;
    let target = oid(ObjectType::BINARY_VALUE, 1);
    let data = rp(target, PropertyIdentifier::PRESENT_VALUE, None);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::READ_PROPERTY,
        data.clone(),
    )
    .await;
    let baseline = dispatch(&plain.server, ConfirmedServiceChoice::READ_PROPERTY, data).await;
    assert_eq!(wire(&response), wire(&baseline));
    let Apdu::ComplexAck(ack) = response else {
        panic!("{response:?}")
    };
    assert_eq!(
        ReadPropertyACK::decode(&ack.service_ack)
            .unwrap()
            .property_value,
        vec![0x91, 0]
    );
    settle().await;
    assert_eq!(
        records(&fixture),
        vec![expected(
            target,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            77,
            0,
            None
        )]
    );
    assert!(records(&plain).is_empty());
    fixture.server.stop().await.unwrap();
    plain.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_rp_known_source_wildcard_device_and_array_identity() {
    let mut fixture = server(read_reporter()).await;
    fixture
        .server
        .device_bindings
        .write()
        .await
        .insert_configured(
            DeviceBinding::local(oid(ObjectType::DEVICE, 30), SOURCE).unwrap(),
            |_| false,
        )
        .unwrap();
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::READ_PROPERTY,
        rp(
            oid(ObjectType::DEVICE, 4194303),
            PropertyIdentifier::OBJECT_LIST,
            Some(0),
        ),
    )
    .await;
    let Apdu::ComplexAck(ack) = response else {
        panic!("{response:?}")
    };
    assert_eq!(
        ReadPropertyACK::decode(&ack.service_ack)
            .unwrap()
            .object_identifier,
        oid(ObjectType::DEVICE, 10)
    );
    settle().await;
    let mut record = expected(
        oid(ObjectType::DEVICE, 10),
        PropertyIdentifier::OBJECT_LIST,
        Some(0),
        77,
        0,
        None,
    );
    record.source_device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 30));
    assert_eq!(records(&fixture), vec![record]);
    fixture.server.stop().await.unwrap();
}

struct ReadProbe {
    reads: Arc<AtomicUsize>,
    failure: fn() -> Error,
}

fn probe_oid() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 9)
}

impl BACnetObject for ReadProbe {
    fn object_identifier(&self) -> ObjectIdentifier {
        probe_oid()
    }
    fn object_name(&self) -> &str {
        "read probe"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::DESCRIPTION,
        ])
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        if property == PropertyIdentifier::PRESENT_VALUE {
            Ok(PropertyValue::Null)
        } else {
            Err((self.failure)())
        }
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        unreachable!("read-only test")
    }
}

async fn add_probe(fixture: &Fixture, failure: fn() -> Error) -> Arc<AtomicUsize> {
    let reads = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(ReadProbe {
            reads: Arc::clone(&reads),
            failure,
        }))
        .unwrap();
    reads
}

#[tokio::test]
async fn audit_reporter_rp_execution_errors_map_result_but_unknown_outcomes_are_silent() {
    type ExecutionCase = (fn() -> Error, Option<(ErrorClass, ErrorCode)>);
    let cases: [ExecutionCase; 5] = [
        (
            || Error::Protocol { class: 2, code: 32 },
            Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        ),
        (
            || Error::Encoding("executed service failure".into()),
            Some((ErrorClass::SERVICES, ErrorCode::OTHER)),
        ),
        (|| Error::Timeout(Duration::from_secs(1)), None),
        (
            || Error::Reject {
                reason: RejectReason::OTHER.to_raw(),
            },
            None,
        ),
        (
            || Error::Abort {
                reason: AbortReason::OTHER.to_raw(),
            },
            None,
        ),
    ];
    for (error, result) in cases {
        let mut fixture = server(read_reporter()).await;
        let reads = add_probe(&fixture, error).await;
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::READ_PROPERTY,
            rp(probe_oid(), PropertyIdentifier::DESCRIPTION, None),
        )
        .await;
        assert!(matches!(response, Apdu::Error(_) | Apdu::Reject(_)));
        settle().await;
        assert_eq!(reads.load(Ordering::Acquire), 1, "no audit value re-read");
        let expected_records = result
            .map(|result| {
                if let Apdu::Error(error) = &response {
                    assert_eq!((error.error_class, error.error_code), result);
                } else {
                    panic!("{response:?}");
                }
                expected(
                    probe_oid(),
                    PropertyIdentifier::DESCRIPTION,
                    None,
                    77,
                    0,
                    Some(result),
                )
            })
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(records(&fixture), expected_records);
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_rpm_expanded_order_inline_errors_and_response_parity() {
    let mut fixture = server(read_reporter()).await;
    let mut plain = server(read_reporter()).await;
    plain.server.config.audit_reporter = None;
    let reads = add_probe(&fixture, || Error::Encoding("mapped inline".into())).await;
    add_probe(&plain, || Error::Encoding("mapped inline".into())).await;
    let missing = oid(ObjectType::BINARY_VALUE, 99);
    let input = oid(ObjectType::ANALOG_INPUT, 1);
    let data = rpm(vec![
        spec(
            probe_oid(),
            &[
                (PropertyIdentifier::ALL, None),
                (PropertyIdentifier::PRESENT_VALUE, None),
            ],
        ),
        spec(input, &[(PropertyIdentifier::PRESENT_VALUE, Some(3))]),
        spec(missing, &[(PropertyIdentifier::OBJECT_NAME, None)]),
        spec(
            oid(ObjectType::DEVICE, 4194303),
            &[(PropertyIdentifier::OBJECT_LIST, Some(0))],
        ),
    ]);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        data.clone(),
    )
    .await;
    let baseline = dispatch(
        &plain.server,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        data,
    )
    .await;
    assert_eq!(wire(&response), wire(&baseline));
    let Apdu::ComplexAck(ack) = response else {
        panic!("{response:?}")
    };
    let ack = ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap();
    let outcomes: Vec<_> = ack
        .list_of_read_access_results
        .iter()
        .flat_map(|spec| {
            spec.list_of_results.iter().map(move |result| {
                (
                    spec.object_identifier,
                    result.property_identifier,
                    result.property_array_index,
                    result.error,
                )
            })
        })
        .collect();
    let expected_outcomes = vec![
        (probe_oid(), PropertyIdentifier::PRESENT_VALUE, None, None),
        (
            probe_oid(),
            PropertyIdentifier::DESCRIPTION,
            None,
            Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        ),
        (probe_oid(), PropertyIdentifier::PRESENT_VALUE, None, None),
        (
            input,
            PropertyIdentifier::PRESENT_VALUE,
            Some(3),
            Some((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
        ),
        (
            missing,
            PropertyIdentifier::OBJECT_NAME,
            None,
            Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
        ),
        (
            oid(ObjectType::DEVICE, 4194303),
            PropertyIdentifier::OBJECT_LIST,
            Some(0),
            None,
        ),
    ];
    assert_eq!(outcomes, expected_outcomes);
    assert_eq!(reads.load(Ordering::Acquire), 3);
    settle().await;
    let mut expected_records: Vec<_> = expected_outcomes
        .into_iter()
        .enumerate()
        .map(|(i, (target, property, index, error))| {
            expected(target, property, index, 77, i as u16, error)
        })
        .collect();
    expected_records[5].target_object = Some(oid(ObjectType::DEVICE, 10));
    assert_eq!(records(&fixture), expected_records);
    assert!(records(&plain).is_empty());
    fixture.server.stop().await.unwrap();
    plain.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_read_operation_levels_and_selection_apply_to_rp_and_rpm() {
    let first = oid(ObjectType::BINARY_VALUE, 1);
    let input = oid(ObjectType::ANALOG_INPUT, 1);
    let reporter_oid = oid(ObjectType::AUDIT_REPORTER, 1);
    for (level, bit, selection, targets, pv) in [
        (AuditLevel::NONE, true, None, vec![], false),
        (AuditLevel::AUDIT_ALL, false, None, vec![], false),
        (
            AuditLevel::AUDIT_CONFIG,
            true,
            None,
            vec![first, input, reporter_oid],
            false,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            None,
            vec![first, input, reporter_oid],
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![]),
            vec![reporter_oid],
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::None]),
            vec![reporter_oid],
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::Object(first), Selector::Object(first)]),
            vec![first, reporter_oid],
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::ObjectType(ObjectType::ANALOG_INPUT)]),
            vec![input, reporter_oid],
            true,
        ),
    ] {
        for multiple in [false, true] {
            let mut reporter = read_reporter();
            reporter.set_audit_level(level).unwrap();
            if !bit {
                reporter.set_auditable_operations(AuditOperationFlags::empty());
            }
            reporter.set_monitored_objects(selection.clone());
            reporter.set_audit_priority_filter(BACnetPriorityFilter::empty());
            let mut fixture = server(reporter).await;
            let mut specs = Vec::new();
            let mut expected_records = Vec::new();
            let mut invoke = 77;
            for target in [first, input, reporter_oid] {
                for property in [
                    PropertyIdentifier::DESCRIPTION,
                    PropertyIdentifier::PRESENT_VALUE,
                ] {
                    if !multiple {
                        let _ = dispatch(
                            &fixture.server,
                            ConfirmedServiceChoice::READ_PROPERTY,
                            rp(target, property, None),
                        )
                        .await;
                    }
                    // Reporter has no Present_Value; the returned error still
                    // counts as an ALL-level executed property outcome.
                    let error = (target == reporter_oid
                        && property == PropertyIdentifier::PRESENT_VALUE)
                        .then_some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY));
                    if targets.contains(&target)
                        && (property != PropertyIdentifier::PRESENT_VALUE || pv)
                    {
                        expected_records.push(expected(
                            target,
                            property,
                            None,
                            if multiple { 77 } else { invoke },
                            expected_records.len() as u16,
                            error,
                        ));
                    }
                    specs.push(spec(target, &[(property, None)]));
                    invoke += 1;
                }
            }
            if multiple {
                assert!(matches!(
                    dispatch(
                        &fixture.server,
                        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                        rpm(specs)
                    )
                    .await,
                    Apdu::ComplexAck(_)
                ));
            }
            settle().await;
            assert_eq!(
                records(&fixture),
                expected_records,
                "level={level:?} bit={bit} multiple={multiple}"
            );
            fixture.server.stop().await.unwrap();
        }
    }
}
