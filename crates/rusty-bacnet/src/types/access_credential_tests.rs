use std::borrow::Cow;

use super::*;
use bacnet_encoding::constructed::encode_assigned_access_rights;
use bacnet_objects::access_control::{
    AccessCredentialObject, AccessPointObject, AccessRightsObject, evaluate_access_rights,
};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

// The public Rust object trait allows an application to serve invalid or
// unreadable rows. Python cannot configure these fixtures through its setters.
struct Fixture {
    oid: ObjectIdentifier,
    name: String,
    assignments: Option<PropertyValue>,
}

impl BACnetObject for Fixture {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        &self.name
    }
    fn read_property(&self, property: P, _index: Option<u32>) -> Result<PropertyValue, Error> {
        if property == P::ASSIGNED_ACCESS_RIGHTS
            && let Some(value) = &self.assignments
        {
            return Ok(value.clone());
        }
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))
    }
    fn write_property(
        &mut self,
        _property: P,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        Cow::Borrowed(&[])
    }
}

#[test]
fn access_credential_result_keeps_every_unresolved_reason_and_original_reference() {
    let card = oid(ObjectType::ACCESS_CREDENTIAL, 1);
    let point = oid(ObjectType::ACCESS_POINT, 1);
    let rights = oid(ObjectType::ACCESS_RIGHTS, 1);
    let missing = oid(ObjectType::ACCESS_RIGHTS, 77);
    let unreadable = oid(ObjectType::ACCESS_RIGHTS, 88);
    let wrong = oid(ObjectType::ANALOG_VALUE, 1);
    let remote = oid(ObjectType::DEVICE, 99);
    let wildcard = oid(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AccessPointObject::new(1, "Point").unwrap()))
        .unwrap();
    let mut allow = AccessRightsObject::new(1, "Allow").unwrap();
    allow
        .set_positive_access_rules(vec![
            BACnetAccessRule::new(None, None, false),
            BACnetAccessRule::new(None, None, true),
        ])
        .unwrap();
    db.add(Box::new(allow)).unwrap();
    db.add(Box::new(Fixture {
        oid: unreadable,
        name: "Unreadable".into(),
        assignments: None,
    }))
    .unwrap();
    let references = [
        (wrong.into(), false),
        (
            BACnetDeviceObjectReference {
                device_identifier: Some(remote),
                object_identifier: rights,
            },
            true,
        ),
        (
            oid(ObjectType::ACCESS_RIGHTS, ObjectIdentifier::MAX_INSTANCE).into(),
            true,
        ),
        (
            BACnetDeviceObjectReference {
                device_identifier: Some(wildcard),
                object_identifier: rights,
            },
            true,
        ),
        (wrong.into(), true),
        (missing.into(), true),
        (unreadable.into(), true),
        (rights.into(), true),
        (missing.into(), true),
    ];
    let values = references
        .into_iter()
        .map(|(reference, enable)| {
            let mut bytes = BytesMut::new();
            encode_assigned_access_rights(
                &mut bytes,
                &BACnetAssignedAccessRights {
                    assigned_access_rights: reference,
                    enable,
                },
            );
            PropertyValue::ApplicationData(bytes.to_vec())
        })
        .collect();
    db.add(Box::new(Fixture {
        oid: card,
        name: "Custom credential".into(),
        assignments: Some(PropertyValue::List(values)),
    }))
    .unwrap();
    let result = evaluate_access_rights(&db, card, point).unwrap();
    Python::initialize();
    Python::attach(|py| {
        let locals = PyDict::new(py);
        for (name, id) in [
            ("rights", rights),
            ("missing", missing),
            ("wrong", wrong),
            ("unreadable", unreadable),
            ("remote", remote),
            ("wildcard", wildcard),
        ] {
            locals
                .set_item(name, PyObjectIdentifier::from_rust(id))
                .unwrap();
        }
        locals
            .set_item(
                "result",
                AccessRightsValue(result).into_pyobject(py).unwrap(),
            )
            .unwrap();
        py.run(
            c"assert result == {\n\
            'decision': 'granted', 'access_event': None,\n\
            'rule': {'access_rights': rights, 'kind': 'positive', 'index': 2},\n\
            'unresolved': [\n\
                {'index': 2, 'reference': (remote, rights), 'reason': 'remote'},\n\
                {'index': 4, 'reference': (wildcard, rights), 'reason': 'wildcard_device'},\n\
                {'index': 5, 'reference': wrong, 'reason': 'not_access_rights'},\n\
                {'index': 6, 'reference': missing, 'reason': 'missing'},\n\
                {'index': 7, 'reference': unreadable, 'reason': 'unreadable'},\n\
                {'index': 9, 'reference': missing, 'reason': 'missing'}]}",
            None,
            Some(&locals),
        )
        .unwrap();
    });
}

#[test]
fn access_credential_result_maps_denial_rule_and_nullable_exemption_fields() {
    let card = oid(ObjectType::ACCESS_CREDENTIAL, 1);
    let point = oid(ObjectType::ACCESS_POINT, 1);
    let rights = oid(ObjectType::ACCESS_RIGHTS, 1);
    for exempt in [false, true] {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(AccessPointObject::new(1, "Point").unwrap()))
            .unwrap();
        let mut deny = AccessRightsObject::new(1, "Deny").unwrap();
        deny.set_negative_access_rules(vec![BACnetAccessRule::new(None, None, true)])
            .unwrap();
        db.add(Box::new(deny)).unwrap();
        let mut credential = AccessCredentialObject::new(1, "Credential").unwrap();
        credential
            .set_assigned_access_rights(vec![BACnetAssignedAccessRights {
                assigned_access_rights: rights.into(),
                enable: true,
            }])
            .unwrap();
        if exempt {
            credential
                .set_authorization_exemptions(Some(vec![AuthorizationExemption::ACCESS_RIGHTS]))
                .unwrap();
        }
        db.add(Box::new(credential)).unwrap();
        let value = evaluate_access_rights(&db, card, point).unwrap();
        Python::initialize();
        Python::attach(|py| {
            let locals = PyDict::new(py);
            locals.set_item("exempt", exempt).unwrap();
            locals
                .set_item("rights", PyObjectIdentifier::from_rust(rights))
                .unwrap();
            locals
                .set_item(
                    "result",
                    AccessRightsValue(value).into_pyobject(py).unwrap(),
                )
                .unwrap();
            py.run(c"expected = {'decision': 'exempt', 'access_event': None, 'rule': None, 'unresolved': []} if exempt else {'decision': 'denied', 'access_event': 134, 'rule': {'access_rights': rights, 'kind': 'negative', 'index': 1}, 'unresolved': []}\nassert result == expected", None, Some(&locals)).unwrap();
        });
    }
}

#[test]
fn access_credential_converters_keep_fixed_width_and_native_range_boundaries() {
    Python::initialize();
    Python::attach(|py| {
        let values = PyList::new(py, [0_u32, 6, 7, 63, 64, 255, 256, u32::MAX]).unwrap();
        let converted = authorization_exemptions_from_py(values.as_any()).unwrap();
        assert_eq!(
            converted
                .into_iter()
                .map(AuthorizationExemption::to_raw)
                .collect::<Vec<_>>(),
            [0, 6, 7, 63, 64, 255, 256, u32::MAX]
        );
        for value in [
            py.eval(c"True", None, None).unwrap(),
            py.eval(c"2.0", None, None).unwrap(),
        ] {
            let error =
                authorization_exemptions_from_py(PyList::new(py, [value]).unwrap().as_any())
                    .unwrap_err();
            assert!(error.is_instance_of::<PyTypeError>(py));
        }
        for value in [-1_i64, 4_294_967_296] {
            let error =
                authorization_exemptions_from_py(PyList::new(py, [value]).unwrap().as_any())
                    .unwrap_err();
            assert!(error.is_instance_of::<pyo3::exceptions::PyOverflowError>(py));
        }
        let assignment = PyDict::new(py);
        assignment
            .set_item(
                "assigned_access_rights",
                PyObjectIdentifier::from_rust(oid(ObjectType::ACCESS_RIGHTS, 7)),
            )
            .unwrap();
        assignment.set_item("enable", false).unwrap();
        let list = PyList::new(py, [&assignment]).unwrap();
        assert_eq!(
            assigned_access_rights_from_py(list.as_any()).unwrap(),
            vec![BACnetAssignedAccessRights {
                assigned_access_rights: oid(ObjectType::ACCESS_RIGHTS, 7).into(),
                enable: false,
            }]
        );
        assignment.set_item("enable", 0).unwrap();
        let error = assigned_access_rights_from_py(list.as_any()).unwrap_err();
        assert!(error.is_instance_of::<PyTypeError>(py));
        assert!(
            error
                .to_string()
                .contains("assigned_access_rights[0].enable")
        );
    });
}
