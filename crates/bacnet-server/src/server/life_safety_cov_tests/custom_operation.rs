//! Custom LifeSafetyOperation contract exercised through confirmed dispatch.
use super::*;
use bacnet_objects::traits::{BACnetObject, LifeSafetyOperationEffect, LifeSafetyOperationOutcome};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Copy)]
enum Behavior {
    Apply,
    ApplyWithoutReadback,
    AlreadyApplied,
    Refuse,
}

struct CustomPoint {
    name: String,
    oid: ObjectIdentifier,
    present: u32,
    tracking: u32,
    behavior: Behavior,
    calls: Arc<AtomicUsize>,
}
impl CustomPoint {
    fn execute(&mut self) -> Result<LifeSafetyOperationOutcome, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behavior {
            Behavior::ApplyWithoutReadback => {
                self.behavior = Behavior::AlreadyApplied;
                Ok(LifeSafetyOperationOutcome {
                    effect: LifeSafetyOperationEffect::Applied,
                    changed_properties: vec![],
                })
            }
            Behavior::Refuse => Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw().into(),
                code: ErrorCode::INVALID_OPERATION_IN_THIS_STATE.to_raw().into(),
            }),
            Behavior::AlreadyApplied => Ok(LifeSafetyOperationOutcome {
                effect: LifeSafetyOperationEffect::AlreadyApplied,
                changed_properties: vec![],
            }),
            Behavior::Apply => {
                self.present = 1;
                self.tracking = 2;
                self.behavior = Behavior::AlreadyApplied;
                Ok(LifeSafetyOperationOutcome {
                    effect: LifeSafetyOperationEffect::Applied,
                    changed_properties: vec![
                        PropertyIdentifier::TRACKING_VALUE,
                        PropertyIdentifier::PRESENT_VALUE,
                    ],
                })
            }
        }
    }
}
impl BACnetObject for CustomPoint {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        &self.name
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
            PropertyIdentifier::STATUS_FLAGS,
        ])
    }
    fn read_property(&self, p: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        match p {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Enumerated(self.tracking))
            }
            p if p == PropertyIdentifier::SILENCED
                || p == PropertyIdentifier::OPERATION_EXPECTED =>
            {
                Ok(PropertyValue::Enumerated(0))
            }
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            }),
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw().into(),
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw().into(),
            }),
        }
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw().into(),
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw().into(),
        })
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn supports_cov_property(&self, p: PropertyIdentifier) -> bool {
        [
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::STATUS_FLAGS,
        ]
        .contains(&p)
    }
    fn apply_life_safety_operation(
        &mut self,
        _: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        self.execute()
    }
}
fn add(db: &mut ObjectDatabase, instance: u32, behavior: Behavior) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    db.add(Box::new(CustomPoint {
        name: format!("custom point {instance}"),
        oid: ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, instance).unwrap(),
        present: 0,
        tracking: 0,
        behavior,
        calls: calls.clone(),
    }))
    .unwrap();
    calls
}
fn request(oid: Option<ObjectIdentifier>) -> Bytes {
    let mut encoded = BytesMut::new();
    LifeSafetyOperationRequest {
        requesting_process_identifier: 9,
        requesting_source: "custom contract".into(),
        request: LifeSafetyOperation::SILENCE,
        object_identifier: oid,
    }
    .encode(&mut encoded)
    .unwrap();
    encoded.freeze()
}
fn subscriptions(oid: ObjectIdentifier) -> Vec<CovSubscription> {
    [
        None,
        Some(PropertyIdentifier::TRACKING_VALUE),
        Some(PropertyIdentifier::SILENCED),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, p)| {
        let mut sub = subscription(p, CovNotificationKind::Single, (i + 1) as u32);
        sub.monitored_object_identifier = oid;
        sub
    })
    .collect()
}
fn assert_notifications(apdus: &[Apdu], oid: ObjectIdentifier, invoke_id: u8) {
    assert!(matches!(&apdus[0], Apdu::SimpleAck(ack) if ack.invoke_id == invoke_id));
    assert_eq!(apdus.len(), 3, "ACK precedes exactly the whole-object and Tracking_Value COV; unchanged Silenced is silent");
    let mut payloads = Vec::new();
    for apdu in &apdus[1..] {
        let Apdu::UnconfirmedRequest(request) = apdu else {
            panic!("expected COV")
        };
        let notification = COVNotificationRequest::decode(&request.service_request).unwrap();
        assert_eq!(notification.monitored_object_identifier, oid);
        payloads.push(
            notification
                .list_of_values
                .into_iter()
                .map(|v| (v.property_identifier, v.value))
                .collect::<Vec<_>>(),
        );
    }
    assert!(payloads.contains(&vec![
        (PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]),
        (PropertyIdentifier::STATUS_FLAGS, vec![0x82, 4, 0])
    ]));
    assert!(payloads.contains(&vec![
        (PropertyIdentifier::TRACKING_VALUE, vec![0x91, 2]),
        (PropertyIdentifier::STATUS_FLAGS, vec![0x82, 4, 0])
    ]));
}

#[tokio::test]
async fn custom_sole_hook_target_delivers_exact_cov_after_ack_once() {
    let mut db = clocked_test_database();
    let calls = add(&mut db, 1, Behavior::Apply);
    let fixture = DispatchFixture::new(db, subscriptions(point_oid())).await;
    let request = request(Some(point_oid()));
    fixture
        .dispatch(
            0x61,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            request.clone(),
        )
        .await;
    assert_notifications(&fixture.take_apdus(), point_oid(), 0x61);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fixture
        .dispatch(
            0x61,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            request.clone(),
        )
        .await;
    let replay = fixture.take_apdus();
    assert_eq!(replay.len(), 1);
    assert!(matches!(&replay[0], Apdu::SimpleAck(ack) if ack.invoke_id == 0x61));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "replay must not invoke the object"
    );
    fixture
        .dispatch(0x62, ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, request)
        .await;
    let no_op = fixture.take_apdus();
    assert_eq!(no_op.len(), 1);
    assert!(matches!(&no_op[0], Apdu::SimpleAck(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn custom_all_applicable_preserves_successes_and_exact_cov() {
    let mut db = clocked_test_database();
    // Insert out of order; execution and COV delivery retain sorted object order.
    let fourth = add(&mut db, 4, Behavior::Apply);
    let failed = add(&mut db, 3, Behavior::Refuse);
    let unchanged = add(&mut db, 2, Behavior::AlreadyApplied);
    let first = add(&mut db, 1, Behavior::Apply);
    let subscriptions = (1..=4).flat_map(|i| {
        subscriptions(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, i).unwrap())
    });
    let fixture = DispatchFixture::new(db, subscriptions).await;
    fixture
        .dispatch(
            0x63,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            request(None),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 5);
    assert_notifications(&apdus[..3], point_oid(), 0x63);
    assert_notifications(
        &[apdus[0].clone(), apdus[3].clone(), apdus[4].clone()],
        ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 4).unwrap(),
        0x63,
    );
    for calls in [first, unchanged, failed, fourth] {
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    let db = fixture.db.read().await;
    for i in 1..=4 {
        let object = db
            .get(&ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, i).unwrap())
            .unwrap();
        let applied = i == 1 || i == 4;
        assert_eq!(
            object
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(u32::from(applied))
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::TRACKING_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(if applied { 2 } else { 0 })
        );
    }
}

#[tokio::test]
async fn custom_refusal_and_already_applied_are_silent_and_unchanged() {
    for behavior in [Behavior::Refuse, Behavior::AlreadyApplied] {
        let mut db = clocked_test_database();
        let calls = add(&mut db, 1, behavior);
        let fixture = DispatchFixture::new(db, subscriptions(point_oid())).await;
        fixture
            .dispatch(
                0x64,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
                request(Some(point_oid())),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), 1, "no COV for refused or unchanged operations");
        if matches!(behavior, Behavior::Refuse) {
            assert!(
                matches!(&apdus[0], Apdu::Error(error) if error.invoke_id == 0x64 && error.error_class == ErrorClass::OBJECT && error.error_code == ErrorCode::INVALID_OPERATION_IN_THIS_STATE)
            );
        } else {
            assert!(matches!(&apdus[0], Apdu::SimpleAck(ack) if ack.invoke_id == 0x64));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let db = fixture.db.read().await;
        for property in [
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
        ] {
            assert_eq!(
                db.get(&point_oid())
                    .unwrap()
                    .read_property(property, None)
                    .unwrap(),
                PropertyValue::Enumerated(0)
            );
        }
    }
}

#[test]
fn custom_outcome_preserves_ordered_exact_properties_at_handler_boundary() {
    let mut db = clocked_test_database();
    add(&mut db, 1, Behavior::Apply);
    let request = LifeSafetyOperationRequest::decode(&request(Some(point_oid()))).unwrap();
    let result = crate::handlers::handle_life_safety_operation(&mut db, &request).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].object_identifier, point_oid());
    assert_eq!(
        result[0].changed_properties,
        vec![
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::PRESENT_VALUE
        ]
    );
}

#[tokio::test]
async fn applied_without_readback_deltas_succeeds_without_cov() {
    for target in [Some(point_oid()), None] {
        let mut db = clocked_test_database();
        let calls = add(&mut db, 1, Behavior::ApplyWithoutReadback);
        let fixture = DispatchFixture::new(db, subscriptions(point_oid())).await;
        fixture
            .dispatch(
                0x65,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
                request(target),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), 1);
        assert!(matches!(&apdus[0], Apdu::SimpleAck(ack) if ack.invoke_id == 0x65));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let mut db = fixture.db.write().await;
        let object = db.get_mut(&point_oid()).unwrap();
        let next = object
            .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
            .unwrap();
        assert_eq!(
            next.effect,
            LifeSafetyOperationEffect::AlreadyApplied,
            "private operation state was committed despite empty readback deltas"
        );
        assert!(next.changed_properties.is_empty());
        for property in [
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
        ] {
            assert_eq!(
                object.read_property(property, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
        }
    }
}
