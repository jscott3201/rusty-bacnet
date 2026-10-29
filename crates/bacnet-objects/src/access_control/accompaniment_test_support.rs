//! Independent fixtures for the local accompaniment contract.
use super::*;
use crate::database::ObjectDatabase;
use bacnet_types::constructed::{BACnetAccessRule, BACnetAssignedAccessRights};
use bacnet_types::enums::{AuthorizationExemption, ErrorClass, ErrorCode};
use std::time::Duration;

pub(super) fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}
pub(super) fn point() -> ObjectIdentifier {
    oid(ObjectType::ACCESS_POINT, 1)
}
pub(super) fn credential(n: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_CREDENTIAL, n)
}
pub(super) fn rights(n: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_RIGHTS, n)
}
pub(super) fn database(requirement: Option<BACnetDeviceObjectReference>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AccessPointObject::new(1, "point").unwrap()))
        .unwrap();
    for n in 1..=3 {
        let mut ar = AccessRightsObject::new(n, format!("rights-{n}")).unwrap();
        ar.set_positive_access_rules([BACnetAccessRule::new(None, Some(point().into()), true)])
            .unwrap();
        if n == 1 {
            ar.set_accompaniment(requirement.clone()).unwrap();
        }
        db.add(Box::new(ar)).unwrap();
        let mut card = AccessCredentialObject::new(n, format!("card-{n}")).unwrap();
        card.set_assigned_access_rights(vec![BACnetAssignedAccessRights {
            assigned_access_rights: rights(n).into(),
            enable: true,
        }])
        .unwrap();
        db.add(Box::new(card)).unwrap();
    }
    db
}
pub(super) fn original() -> CredentialPresentation {
    CredentialPresentation {
        sequence: 10,
        credential: credential(1),
        point: point(),
        at: Duration::from_secs(100),
    }
}
pub(super) fn request(witnesses: &[AccompanimentWitness]) -> AccompanimentRequest<'_> {
    AccompanimentRequest {
        original: original(),
        now: Duration::from_secs(101),
        witnesses,
        timing: AccompanimentTiming {
            order: AccompanimentOrder::After,
            fallback_period: Duration::from_secs(10),
            deadline: AccompanimentDeadline::Inclusive,
        },
        max_links: 16,
    }
}
pub(super) fn companion(n: u32) -> CredentialPresentation {
    CredentialPresentation {
        sequence: 11,
        credential: credential(n),
        point: point(),
        at: Duration::from_secs(101),
    }
}
pub(super) fn presented(n: u32) -> AccompanimentWitness {
    AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Presented(companion(n)),
    }
}

pub(super) fn assignment(
    reference: impl Into<BACnetDeviceObjectReference>,
    enabled: bool,
) -> BACnetAssignedAccessRights {
    BACnetAssignedAccessRights {
        assigned_access_rights: reference.into(),
        enable: enabled,
    }
}
pub(super) fn set_assignments(
    db: &mut ObjectDatabase,
    n: u32,
    assignments: Vec<BACnetAssignedAccessRights>,
    exempt: bool,
) {
    db.remove(&credential(n)).unwrap();
    let mut object = AccessCredentialObject::new(n, format!("card-{n}")).unwrap();
    object.set_assigned_access_rights(assignments).unwrap();
    if exempt {
        object
            .set_authorization_exemptions(Some(vec![AuthorizationExemption::ACCESS_RIGHTS]))
            .unwrap();
    }
    db.add(Box::new(object)).unwrap();
}
pub(super) fn set_rights(
    db: &mut ObjectDatabase,
    n: u32,
    requirement: Option<BACnetDeviceObjectReference>,
    positive: Vec<BACnetAccessRule>,
    negative: Vec<BACnetAccessRule>,
    enabled: bool,
) {
    db.remove(&rights(n)).unwrap();
    let mut object = AccessRightsObject::new(n, format!("rights-{n}")).unwrap();
    object.set_positive_access_rules(positive).unwrap();
    object.set_negative_access_rules(negative).unwrap();
    object.set_accompaniment(requirement).unwrap();
    object.set_enable(enabled);
    db.add(Box::new(object)).unwrap();
}
pub(super) fn anytime() -> BACnetAccessRule {
    BACnetAccessRule::new(None, Some(point().into()), true)
}
pub(super) fn user(n: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_USER, n)
}
pub(super) fn add_user(db: &mut ObjectDatabase, entries: Vec<BACnetDeviceObjectReference>) {
    let mut user = AccessUserObject::new(1, "owner").unwrap();
    user.set_credentials(entries).unwrap();
    db.add(Box::new(user)).unwrap();
}
pub(super) fn device(n: u32) -> ObjectIdentifier {
    oid(ObjectType::DEVICE, n)
}
pub(super) fn qualified(
    object: ObjectIdentifier,
    device: ObjectIdentifier,
) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        object_identifier: object,
        device_identifier: Some(device),
    }
}
pub(super) fn add_device(db: &mut ObjectDatabase) {
    use crate::device::{DeviceConfig, DeviceObject};
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::new(100, 0)
                .unwrap()
                .with_name("local"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
}
pub(super) fn evaluate(
    db: &ObjectDatabase,
    witnesses: &[AccompanimentWitness],
) -> AccompanimentEvaluation {
    evaluate_accompaniment(db, request(witnesses)).unwrap()
}
pub(super) fn by(presentation: CredentialPresentation) -> AccompanimentEventIntent {
    AccompanimentEventIntent {
        original: original(),
        access_event: AccessEvent::ACCOMPANIMENT_BY,
        presentation,
    }
}
pub(super) fn incorrect() -> AccompanimentEventIntent {
    AccompanimentEventIntent {
        original: original(),
        access_event: AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT,
        presentation: original(),
    }
}
pub(super) fn assert_incorrect(
    result: &AccompanimentEvaluation,
    attempted: CredentialPresentation,
) {
    assert!(
        matches!(
            result.outcome,
            AccompanimentOutcome::DeniedIncorrectCompanion { .. }
        ),
        "{result:?}"
    );
    assert_eq!(result.events, [by(attempted), incorrect()]);
}

/// A custom object boundary: only named property reads differ from the native
/// object. Panics prove a short circuit avoided a forbidden read or any write.
pub(super) enum Read {
    Value(PropertyValue),
    Failure(ErrorClass, ErrorCode),
    Forbidden,
}
struct Overridden {
    inner: Box<dyn BACnetObject>,
    property: PropertyIdentifier,
    read: Read,
    advertised: bool,
}
impl BACnetObject for Overridden {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property != self.property {
            return self.inner.read_property(property, index);
        }
        match &self.read {
            Read::Value(value) => Ok(value.clone()),
            Read::Failure(class, code) => Err(Error::protocol(*class, *code)),
            Read::Forbidden => panic!("forbidden property read: {property:?}"),
        }
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("pure evaluator attempted a write")
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let mut properties = self.inner.property_list().into_owned();
        properties.retain(|property| *property != self.property);
        if self.advertised {
            properties.push(self.property);
        }
        Cow::Owned(properties)
    }
}
pub(super) fn override_read(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    read: Read,
    advertised: bool,
) {
    let inner = db.remove(&oid).unwrap().unwrap();
    db.add(Box::new(Overridden {
        inner,
        property,
        read,
        advertised,
    }))
    .unwrap();
}
pub(super) fn encoded_reference(reference: &BACnetDeviceObjectReference) -> PropertyValue {
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_device_object_reference(&mut bytes, reference);
    PropertyValue::ApplicationData(bytes.to_vec())
}
