//! The Access Rights check (#1331): exemption, gathering and its enable
//! flags, negative rules before positive ones, the denial each outcome
//! carries, unresolved assignments and the caller's errors. The time-range
//! and location rules are in `rights_evaluation_term_tests.rs`.

use std::borrow::Cow;

use bacnet_encoding::constructed::{encode_access_rule, encode_assigned_access_rights};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;

use super::*;
use crate::access_control::{
    AccessCredentialObject, AccessPointObject, AccessRightsObject, AccessZoneObject,
};
use crate::device::{DeviceConfig, DeviceObject};

/// The Device the test databases speak for.
pub(super) const DEVICE: u32 = 100;
/// Another device, whose objects the evaluator never reads.
pub(super) const OTHER_DEVICE: u32 = 99;
pub(super) const UNSPECIFIED: u32 = ObjectIdentifier::MAX_INSTANCE;

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub(super) fn point(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_POINT, instance)
}

pub(super) fn zone(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_ZONE, instance)
}

pub(super) fn rights(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_RIGHTS, instance)
}

pub(super) fn credential(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ACCESS_CREDENTIAL, instance)
}

pub(super) fn device(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::DEVICE, instance)
}

/// `object` qualified with `device_instance`'s Device.
pub(super) fn in_device(
    device_instance: u32,
    object: ObjectIdentifier,
) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(device(device_instance)),
        object_identifier: object,
    }
}

/// `object`'s Present_Value in this device.
pub(super) fn present_value(object: ObjectIdentifier) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        object,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )
}

/// An enabled rule for `location`, or for every location, at any time.
pub(super) fn anytime(location: Option<ObjectIdentifier>) -> BACnetAccessRule {
    BACnetAccessRule::new(None, location.map(Into::into), true)
}

/// An enabled rule for `location` while `time_range` reads TRUE.
pub(super) fn during(
    time_range: BACnetDeviceObjectPropertyReference,
    location: Option<ObjectIdentifier>,
) -> BACnetAccessRule {
    BACnetAccessRule::new(Some(time_range), location.map(Into::into), true)
}

pub(super) fn disabled(mut rule: BACnetAccessRule) -> BACnetAccessRule {
    rule.enable = false;
    rule
}

pub(super) fn assigned(
    reference: BACnetDeviceObjectReference,
    enable: bool,
) -> BACnetAssignedAccessRights {
    BACnetAssignedAccessRights {
        assigned_access_rights: reference,
        enable,
    }
}

/// Device 100, Access Points 1 and 2, and Access Zone 1, entered through
/// Point 1 and left through Point 2.
pub(super) fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: DEVICE,
            name: "DEV".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for instance in [1, 2] {
        db.add(Box::new(
            AccessPointObject::new(instance, format!("AP-{instance}")).unwrap(),
        ))
        .unwrap();
    }
    let mut lobby = AccessZoneObject::new(1, "AZ-1").unwrap();
    lobby.set_entry_points([point(1)]).unwrap();
    lobby.set_exit_points([point(2)]).unwrap();
    db.add(Box::new(lobby)).unwrap();
    db
}

/// Add Access Rights `instance` with these rules, enabled.
pub(super) fn add_rights(
    db: &mut ObjectDatabase,
    instance: u32,
    positive: Vec<BACnetAccessRule>,
    negative: Vec<BACnetAccessRule>,
) {
    let mut object = AccessRightsObject::new(instance, format!("AR-{instance}")).unwrap();
    object.set_positive_access_rules(positive).unwrap();
    object.set_negative_access_rules(negative).unwrap();
    db.add(Box::new(object)).unwrap();
}

/// Add Credential 1 holding `elements`, each enabled.
pub(super) fn add_credential(db: &mut ObjectDatabase, elements: &[BACnetDeviceObjectReference]) {
    add_credential_with(
        db,
        elements.iter().map(|r| assigned(r.clone(), true)).collect(),
        None,
    );
}

pub(super) fn add_credential_with(
    db: &mut ObjectDatabase,
    elements: Vec<BACnetAssignedAccessRights>,
    exemptions: Option<Vec<AuthorizationExemption>>,
) {
    let mut object = AccessCredentialObject::new(1, "CRED-1").unwrap();
    object.set_assigned_access_rights(elements).unwrap();
    object.set_authorization_exemptions(exemptions).unwrap();
    db.add(Box::new(object)).unwrap();
}

/// Credential 1 presented at `at`.
pub(super) fn evaluate_at(db: &ObjectDatabase, at: u32) -> AccessRightsEvaluation {
    evaluate_access_rights(db, credential(1), point(at)).unwrap()
}

/// Credential 1 presented at Point 1.
pub(super) fn decision(db: &ObjectDatabase) -> AccessRightsDecision {
    evaluate_at(db, 1).decision
}

pub(super) fn position(
    rights_instance: u32,
    kind: AccessRuleKind,
    index: u32,
) -> AccessRulePosition {
    AccessRulePosition {
        access_rights: rights(rights_instance),
        kind,
        index,
    }
}

pub(super) fn granted(rights_instance: u32, index: u32) -> AccessRightsDecision {
    AccessRightsDecision::Granted {
        rule: position(rights_instance, AccessRuleKind::Positive, index),
    }
}

pub(super) fn denied(
    access_event: AccessEvent,
    rule: Option<AccessRulePosition>,
) -> AccessRightsDecision {
    AccessRightsDecision::Denied { access_event, rule }
}

pub(super) fn no_rights() -> AccessRightsDecision {
    denied(AccessEvent::DENIED_NO_ACCESS_RIGHTS, None)
}

pub(super) fn out_of_time(rights_instance: u32, index: u32) -> AccessRightsDecision {
    denied(
        AccessEvent::DENIED_OUT_OF_TIME_RANGE,
        Some(position(rights_instance, AccessRuleKind::Positive, index)),
    )
}

/// An application's own object: fixed values, no writes.
pub(super) struct Stub {
    oid: ObjectIdentifier,
    name: String,
    values: Vec<(PropertyIdentifier, PropertyValue)>,
}

impl Stub {
    pub(super) fn new(
        oid: ObjectIdentifier,
        values: Vec<(PropertyIdentifier, PropertyValue)>,
    ) -> Box<Self> {
        Box::new(Self {
            oid,
            name: format!("STUB-{oid:?}"),
            values,
        })
    }
}

impl BACnetObject for Stub {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.values
            .iter()
            .find(|(served, _)| *served == property)
            .map(|(_, value)| value.clone())
            .ok_or_else(common::unknown_property_error)
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

/// Rules as an Access Rights object serves them.
pub(super) fn served_rules(rules: &[BACnetAccessRule]) -> PropertyValue {
    PropertyValue::List(
        rules
            .iter()
            .map(|rule| {
                let mut buf = BytesMut::new();
                encode_access_rule(&mut buf, rule);
                PropertyValue::ApplicationData(buf.to_vec())
            })
            .collect(),
    )
}

/// A stand-in Access Rights object serving `enable` and `positive`, with no
/// negative rules: what an application's own object might serve, unchecked.
pub(super) fn stub_rights(
    instance: u32,
    enable: PropertyValue,
    positive: &[BACnetAccessRule],
) -> Box<Stub> {
    Stub::new(
        rights(instance),
        vec![
            (PropertyIdentifier::LOG_ENABLE, enable),
            (
                PropertyIdentifier::POSITIVE_ACCESS_RULES,
                served_rules(positive),
            ),
            (PropertyIdentifier::NEGATIVE_ACCESS_RULES, served_rules(&[])),
        ],
    )
}

fn assert_unknown_object(result: Result<AccessRightsEvaluation, Error>) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::OBJECT.to_raw() as u32);
            assert_eq!(code, ErrorCode::UNKNOWN_OBJECT.to_raw() as u32);
        }
        other => panic!("expected OBJECT / UNKNOWN_OBJECT, got {other:?}"),
    }
}

#[test]
fn a_credential_with_no_rights_has_no_access_rights() {
    let mut db = database();
    add_credential(&mut db, &[]);
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, no_rights());
    assert!(evaluation.unresolved.is_empty());
    assert!(!evaluation.decision.allows());
    assert_eq!(
        evaluation.decision.access_event(),
        Some(AccessEvent::DENIED_NO_ACCESS_RIGHTS)
    );
    assert_eq!(evaluation.decision.rule(), None);
}

#[test]
fn the_first_positive_rule_that_holds_grants() {
    let mut db = database();
    // Rule 1 covers Point 2 only; rule 2 covers Point 1; rule 3 everywhere.
    add_rights(
        &mut db,
        1,
        vec![
            anytime(Some(point(2))),
            anytime(Some(point(1))),
            anytime(None),
        ],
        vec![],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 2));
    assert!(decision(&db).allows());
    assert_eq!(
        decision(&db).rule(),
        Some(position(1, AccessRuleKind::Positive, 2))
    );
    assert_eq!(decision(&db).access_event(), None);
    assert_eq!(evaluate_at(&db, 2).decision, granted(1, 1));
}

#[test]
fn negative_rules_of_every_object_come_before_any_positive_rule() {
    let mut db = database();
    // The credential's first Access Rights grants everywhere; its second
    // bars Point 1. The second's negative rule still wins at Point 1.
    add_rights(&mut db, 1, vec![anytime(None)], vec![]);
    add_rights(
        &mut db,
        2,
        vec![],
        vec![anytime(Some(point(2))), anytime(Some(point(1)))],
    );
    add_credential(&mut db, &[rights(1).into(), rights(2).into()]);
    assert_eq!(
        decision(&db),
        denied(
            AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS,
            Some(position(2, AccessRuleKind::Negative, 2))
        )
    );
    // At Point 2 the second object's first negative rule holds instead.
    assert_eq!(
        evaluate_at(&db, 2).decision,
        denied(
            AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS,
            Some(position(2, AccessRuleKind::Negative, 1))
        )
    );
}

#[test]
fn a_negative_rule_names_the_place_it_bars() {
    for (location, event) in [
        (Some(point(1)), AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS),
        (Some(zone(1)), AccessEvent::DENIED_ZONE_NO_ACCESS_RIGHTS),
        // A rule for every location bars this point as well (local
        // reading: the clause names only the point and zone cases).
        (None, AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS),
    ] {
        let mut db = database();
        add_rights(&mut db, 1, vec![anytime(None)], vec![anytime(location)]);
        add_credential(&mut db, &[rights(1).into()]);
        assert_eq!(
            decision(&db),
            denied(event, Some(position(1, AccessRuleKind::Negative, 1))),
            "{location:?}"
        );
    }
}

#[test]
fn a_negative_rule_that_does_not_hold_lets_the_positive_rules_decide() {
    let mut db = database();
    // Barred at Point 2 only, and while an unspecified time range reads
    // FALSE: neither negative rule holds at Point 1.
    let never = present_value(oid(ObjectType::SCHEDULE, UNSPECIFIED));
    add_rights(
        &mut db,
        1,
        vec![anytime(Some(point(1)))],
        vec![anytime(Some(point(2))), during(never, Some(point(1)))],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 1));
}

#[test]
fn out_of_time_range_needs_a_positive_rule_covering_the_point() {
    let never = present_value(oid(ObjectType::SCHEDULE, UNSPECIFIED));
    // Covers Point 1 outside its time range: out of time range, naming the
    // first such rule.
    let mut db = database();
    add_rights(
        &mut db,
        1,
        vec![
            anytime(Some(point(2))),
            during(never.clone(), Some(point(1))),
            during(never.clone(), Some(zone(1))),
        ],
        vec![],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), out_of_time(1, 2));
    assert_eq!(
        decision(&db).access_event(),
        Some(AccessEvent::DENIED_OUT_OF_TIME_RANGE)
    );

    // Rules for other places, in or out of their time range: no rights.
    let mut db = database();
    add_rights(
        &mut db,
        1,
        vec![anytime(Some(point(2))), during(never, Some(point(2)))],
        vec![],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), no_rights());
}

#[test]
fn a_disabled_rule_counts_for_nothing() {
    // A disabled positive rule neither grants nor makes the denial out of
    // time range.
    let never = present_value(oid(ObjectType::SCHEDULE, UNSPECIFIED));
    let mut db = database();
    add_rights(
        &mut db,
        1,
        vec![
            disabled(anytime(None)),
            disabled(during(never, Some(point(1)))),
        ],
        vec![],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), no_rights());

    // A disabled negative rule doesn't deny.
    let mut db = database();
    add_rights(
        &mut db,
        1,
        vec![anytime(None)],
        vec![disabled(anytime(None))],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 1));
}

#[test]
fn a_disabled_access_rights_object_gives_no_rules() {
    let mut db = database();
    let mut off = AccessRightsObject::new(1, "AR-1").unwrap();
    off.set_positive_access_rules([anytime(None)]).unwrap();
    off.set_negative_access_rules([anytime(Some(point(1)))])
        .unwrap();
    off.set_enable(false);
    db.add(Box::new(off)).unwrap();
    add_rights(&mut db, 2, vec![anytime(Some(point(1)))], vec![]);
    add_credential(&mut db, &[rights(1).into(), rights(2).into()]);
    // Neither its negative nor its positive rule counts, and it isn't
    // unresolved: it was found.
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, granted(2, 1));
    assert!(evaluation.unresolved.is_empty());
    assert_eq!(evaluate_at(&db, 2).decision, no_rights());
}

#[test]
fn a_disabled_assignment_gives_no_rules_and_is_not_unresolved() {
    let mut db = database();
    add_rights(&mut db, 1, vec![], vec![anytime(None)]);
    add_rights(&mut db, 2, vec![anytime(None)], vec![]);
    add_credential_with(
        &mut db,
        vec![
            assigned(rights(1).into(), false),
            // Unreachable, but disabled: not looked at.
            assigned(in_device(OTHER_DEVICE, rights(3)), false),
            assigned(rights(77).into(), false),
            assigned(rights(2).into(), true),
        ],
        None,
    );
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, granted(2, 1));
    assert!(evaluation.unresolved.is_empty());
}

#[test]
fn exemption_skips_the_check() {
    let mut db = database();
    add_rights(&mut db, 1, vec![], vec![anytime(None)]);
    add_credential_with(
        &mut db,
        vec![
            assigned(rights(1).into(), true),
            assigned(rights(77).into(), true),
        ],
        Some(vec![
            AuthorizationExemption::PASSBACK,
            AuthorizationExemption::ACCESS_RIGHTS,
        ]),
    );
    // The negative rule would deny and Access Rights 77 is missing, but no
    // rule is read.
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, AccessRightsDecision::Exempt);
    assert!(evaluation.unresolved.is_empty());
    assert!(evaluation.decision.allows());
    assert_eq!(evaluation.decision.access_event(), None);
    assert_eq!(evaluation.decision.rule(), None);
}

#[test]
fn other_exemptions_leave_the_check_in_place() {
    for exemptions in [
        None,
        Some(vec![]),
        Some(vec![
            AuthorizationExemption::PASSBACK,
            AuthorizationExemption::OCCUPANCY_CHECK,
            AuthorizationExemption::LOCKOUT,
        ]),
    ] {
        let mut db = database();
        add_rights(&mut db, 1, vec![anytime(None)], vec![anytime(None)]);
        add_credential_with(
            &mut db,
            vec![assigned(rights(1).into(), true)],
            exemptions.clone(),
        );
        assert_eq!(
            decision(&db),
            denied(
                AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS,
                Some(position(1, AccessRuleKind::Negative, 1))
            ),
            "{exemptions:?}"
        );
    }
}

#[test]
fn unresolved_assignments_are_listed_and_ignored() {
    let mut db = database();
    add_rights(&mut db, 1, vec![anytime(None)], vec![]);
    db.add(stub_rights(5, PropertyValue::Unsigned(1), &[]))
        .unwrap();
    let elements = vec![
        in_device(OTHER_DEVICE, rights(1)),
        rights(77).into(),
        // The unused marker isn't unresolved: it names nothing.
        oid(ObjectType::ACCESS_RIGHTS, UNSPECIFIED).into(),
        in_device(UNSPECIFIED, oid(ObjectType::ACCESS_RIGHTS, UNSPECIFIED)),
        // An Enable that isn't a BOOLEAN.
        rights(5).into(),
        rights(1).into(),
    ];
    add_credential(&mut db, &elements);
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, granted(1, 1));
    let unresolved: Vec<_> = evaluation
        .unresolved
        .iter()
        .map(|entry| (entry.index, entry.reference.clone(), entry.reason))
        .collect();
    assert_eq!(
        unresolved,
        [
            (1, elements[0].clone(), UnresolvedReason::Remote),
            (2, elements[1].clone(), UnresolvedReason::Missing),
            (5, elements[4].clone(), UnresolvedReason::Unreadable),
        ]
    );
}

#[test]
fn an_assignment_naming_another_object_type_is_unresolved() {
    // The built-in credential refuses such an element, so an application's
    // own credential serves it.
    let mut db = database();
    let analog = oid(ObjectType::ANALOG_VALUE, 1);
    let element = assigned(analog.into(), true);
    let mut buf = BytesMut::new();
    encode_assigned_access_rights(&mut buf, &element);
    db.add(Stub::new(
        credential(1),
        vec![(
            PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
            PropertyValue::List(vec![PropertyValue::ApplicationData(buf.to_vec())]),
        )],
    ))
    .unwrap();
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, no_rights());
    assert_eq!(
        evaluation.unresolved,
        [UnresolvedAccessRights {
            index: 1,
            reference: analog.into(),
            reason: UnresolvedReason::NotAccessRights,
        }]
    );
}

#[test]
fn rule_arrays_that_do_not_decode_are_unreadable() {
    let mut db = database();
    db.add(Stub::new(
        rights(1),
        vec![
            (PropertyIdentifier::LOG_ENABLE, PropertyValue::Boolean(true)),
            (
                PropertyIdentifier::POSITIVE_ACCESS_RULES,
                served_rules(&[anytime(None)]),
            ),
            // Negative_Access_Rules as an Unsigned, not rules.
            (
                PropertyIdentifier::NEGATIVE_ACCESS_RULES,
                PropertyValue::Unsigned(0),
            ),
        ],
    ))
    .unwrap();
    add_credential(&mut db, &[rights(1).into()]);
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, no_rights());
    assert_eq!(evaluation.unresolved.len(), 1);
    assert_eq!(
        evaluation.unresolved[0].reason,
        UnresolvedReason::Unreadable
    );
}

#[test]
fn an_assignment_names_this_device_with_its_own_or_the_wildcard_device() {
    for (reference, resolved) in [
        (in_device(DEVICE, rights(1)), true),
        (in_device(UNSPECIFIED, rights(1)), true),
        (in_device(OTHER_DEVICE, rights(1)), false),
    ] {
        let mut db = database();
        add_rights(&mut db, 1, vec![anytime(None)], vec![]);
        add_credential(&mut db, std::slice::from_ref(&reference));
        let evaluation = evaluate_at(&db, 1);
        let expected = if resolved { granted(1, 1) } else { no_rights() };
        assert_eq!(evaluation.decision, expected, "{reference:?}");
        assert_eq!(evaluation.unresolved.is_empty(), resolved, "{reference:?}");
    }
}

#[test]
fn without_a_device_object_a_named_device_is_another_device() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AccessPointObject::new(1, "AP-1").unwrap()))
        .unwrap();
    add_rights(&mut db, 1, vec![anytime(None)], vec![]);
    add_credential(&mut db, &[in_device(DEVICE, rights(1)), rights(1).into()]);
    let evaluation = evaluate_at(&db, 1);
    assert_eq!(evaluation.decision, granted(1, 1));
    assert_eq!(evaluation.unresolved.len(), 1);
    assert_eq!(evaluation.unresolved[0].reason, UnresolvedReason::Remote);
}

#[test]
fn the_credential_and_point_must_be_here_and_of_their_types() {
    let mut db = database();
    add_credential(&mut db, &[]);
    assert_unknown_object(evaluate_access_rights(&db, credential(2), point(1)));
    assert_unknown_object(evaluate_access_rights(&db, credential(1), point(3)));
    // Present, but of another type.
    assert_unknown_object(evaluate_access_rights(&db, point(1), point(1)));
    assert_unknown_object(evaluate_access_rights(&db, credential(1), zone(1)));
    assert!(evaluate_access_rights(&db, credential(1), point(1)).is_ok());
}

#[test]
fn rule_kinds_name_their_arrays() {
    assert_eq!(
        AccessRuleKind::Positive.property(),
        PropertyIdentifier::POSITIVE_ACCESS_RULES
    );
    assert_eq!(
        AccessRuleKind::Negative.property(),
        PropertyIdentifier::NEGATIVE_ACCESS_RULES
    );
}
