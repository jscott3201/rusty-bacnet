//! Checking a credential against the Access Rights it is assigned (Clause
//! 12.34.9; #1331).
//!
//! The access-control objects store and serve the rules: an Access Rights
//! object holds Positive_Access_Rules and Negative_Access_Rules, an Access
//! Credential names its rights in Assigned_Access_Rights, and an Access Zone
//! names its Entry_Points. [`evaluate_access_rights`] reads them and tells the
//! application whether the rules let a credential through the Access Point
//! where it was presented, and if not, which Access_Event value the failure
//! carries.
//!
//! The evaluator is pure. It takes a shared borrow of the database, reads
//! properties the way a peer would see them (`read_property`), and does no
//! I/O. It takes no lock and changes nothing: it doesn't set Access_Event or
//! any other property, and acting on the decision is up to the application.
//! A server application calls it under the database read guard, as in
//! `evaluate_access_rights(&*server.database().read().await, credential,
//! point)`, and drops the guard before acting on the decision.
//!
//! [`evaluate_access_rights`] documents each step. Where Clause 12.34.9
//! leaves the reading to the device, this evaluator takes these:
//!
//! - A time range reads an Enumerated as a BACnetBinaryPV, since a read
//!   value doesn't carry its enumeration: ACTIVE is TRUE and any other value
//!   FALSE. Types the clause doesn't name (REAL, CharacterString and the
//!   rest) read FALSE, so an unexpected value never grants.
//! - A negative rule whose location is ALL denies with
//!   DENIED_POINT_NO_ACCESS_RIGHTS: it bars this point as much as a rule
//!   naming the point does, and the clause names a value only for the point
//!   and zone cases.
//! - A reference whose device member is the wildcard Device instance 4194303
//!   names this device. One naming any other device is never read: its time
//!   range and location are FALSE, and its Assigned_Access_Rights element is
//!   listed as unresolved for the application to weigh.
//! - Assigned_Access_Rights elements that can't be resolved here are left out
//!   of the decision, as Clause 12.35.18 asks, and listed, so that the
//!   application can still deny.

use bacnet_encoding::constructed::{decode_access_rule, decode_assigned_access_rights};
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetAssignedAccessRights, BACnetDeviceObjectPropertyReference,
    BACnetDeviceObjectReference,
};
use bacnet_types::enums::{
    AccessEvent, AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier, AuthorizationExemption,
    BinaryPV, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::rights::unspecified;
use crate::common;
use crate::database::{LocalDevice, ObjectDatabase};
use crate::device_reference::decode_references;
use crate::traits::BACnetObject;

/// What [`evaluate_access_rights`] found for one credential at one Access
/// Point.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessRightsEvaluation {
    /// The outcome of the Access Rights check.
    pub decision: AccessRightsDecision,
    /// The enabled Assigned_Access_Rights elements that gave no rules
    /// because their Access Rights object couldn't be read here, in array
    /// order. The decision leaves them out, as Clause 12.35.18 has the
    /// device do; an application that would rather deny while part of a
    /// credential's rights is out of sight checks that this is empty. Always
    /// empty for [`AccessRightsDecision::Exempt`], which reads no rights.
    pub unresolved: Vec<UnresolvedAccessRights>,
}

/// The outcome of the Access Rights check (Clause 12.34.9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessRightsDecision {
    /// The credential's Authorization_Exemptions lists ACCESS_RIGHTS, so the
    /// check didn't run and counts as passed (Clause 12.35.25).
    Exempt,
    /// No negative rule held, and this positive rule did: the first in
    /// gathering and array order.
    #[non_exhaustive]
    Granted {
        /// The positive rule that granted access.
        rule: AccessRulePosition,
    },
    /// The check failed.
    #[non_exhaustive]
    Denied {
        /// The Access_Event value Clause 12.34.9.2 gives this failure:
        /// DENIED_POINT_NO_ACCESS_RIGHTS or DENIED_ZONE_NO_ACCESS_RIGHTS
        /// for a negative rule, DENIED_OUT_OF_TIME_RANGE, or
        /// DENIED_NO_ACCESS_RIGHTS. The evaluator doesn't write it.
        access_event: AccessEvent,
        /// The rule behind the denial: the negative rule that held, or, for
        /// DENIED_OUT_OF_TIME_RANGE, the first positive rule that covered
        /// the point outside its time range. `None` for
        /// DENIED_NO_ACCESS_RIGHTS.
        rule: Option<AccessRulePosition>,
    },
}

impl AccessRightsDecision {
    /// Whether the check let the credential through: granted or exempt.
    pub fn allows(&self) -> bool {
        matches!(self, Self::Exempt | Self::Granted { .. })
    }

    /// The Access_Event value of a denial, `None` otherwise.
    pub fn access_event(&self) -> Option<AccessEvent> {
        match self {
            Self::Denied { access_event, .. } => Some(*access_event),
            _ => None,
        }
    }

    /// The rule the decision names: the granting rule, or a denial's
    /// rule when it has one.
    pub fn rule(&self) -> Option<AccessRulePosition> {
        match self {
            Self::Granted { rule } => Some(*rule),
            Self::Denied { rule, .. } => *rule,
            Self::Exempt => None,
        }
    }
}

/// Where an access rule sits: its Access Rights object, its array and its
/// place in that array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AccessRulePosition {
    /// The Access Rights object holding the rule.
    pub access_rights: ObjectIdentifier,
    /// Which of the object's two rule arrays holds it.
    pub kind: AccessRuleKind,
    /// The rule's one-based array index, the index a ReadProperty of
    /// [`kind.property()`](AccessRuleKind::property) names it by.
    pub index: u32,
}

/// Which rule array of an Access Rights object a rule comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessRuleKind {
    /// Positive_Access_Rules, the rules that grant access.
    Positive,
    /// Negative_Access_Rules, the rules that deny it.
    Negative,
}

impl AccessRuleKind {
    /// The property holding the rules of this kind.
    pub fn property(self) -> PropertyIdentifier {
        match self {
            Self::Positive => PropertyIdentifier::POSITIVE_ACCESS_RULES,
            Self::Negative => PropertyIdentifier::NEGATIVE_ACCESS_RULES,
        }
    }
}

/// An enabled Assigned_Access_Rights element that gave no rules.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnresolvedAccessRights {
    /// The element's one-based index in Assigned_Access_Rights.
    pub index: u32,
    /// The reference the element holds.
    pub reference: BACnetDeviceObjectReference,
    /// Why it gave no rules.
    pub reason: UnresolvedReason,
}

/// Why an Assigned_Access_Rights element gave no rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnresolvedReason {
    /// It names an object in another device, which this evaluator doesn't
    /// read.
    Remote,
    /// It names an object type other than Access Rights.
    NotAccessRights,
    /// The database holds no object with that identifier.
    Missing,
    /// The Access Rights object's Enable or rule arrays didn't read as their
    /// datatypes. The built-in object always serves them; this covers an
    /// application's own object.
    Unreadable,
}

/// Check the Access Rights rules of `credential`, presented at `point`, as
/// Clause 12.34.9.2 describes.
///
/// # Steps
///
/// 1. **Exemption.** When the credential's Authorization_Exemptions lists
///    ACCESS_RIGHTS, the check doesn't run and counts as passed (Clause
///    12.35.25): the decision is [`AccessRightsDecision::Exempt`], and no
///    rule is read. A credential that doesn't serve the optional row is not
///    exempt.
/// 2. **Gathering.** Assigned_Access_Rights is walked in array order.
///    - An element whose own enable flag is FALSE is skipped, and so is the
///      unused marker, instance 4194303 in the object and in the device when
///      it names one (Clause 12.35.18).
///    - An element naming another device, a missing object or an object that
///      isn't Access Rights gives no rules. Clause 12.35.18 has the device
///      ignore such an element, so the decision does; the element is listed
///      in [`AccessRightsEvaluation::unresolved`] so that an application can
///      deny when part of a credential's rights is out of its sight.
///    - An Access Rights object whose Enable is FALSE gives no rules either
///      (Clause 12.34.8), and isn't listed: it was found.
/// 3. **Negative rules first.** Each enabled negative rule of each gathered
///    object is tried, objects in gathering order and rules in array order,
///    before any positive rule is (Clause 12.34.9.2). The first one that
///    holds denies access, with the Access_Event its location gives (see
///    below).
/// 4. **Positive rules.** Otherwise the first enabled positive rule that
///    holds, in the same order, grants access.
/// 5. **No rule held.** When some enabled positive rule covered this point
///    but its time range was FALSE, the credential could pass here at another
///    time, so the denial is DENIED_OUT_OF_TIME_RANGE. Otherwise it is
///    DENIED_NO_ACCESS_RIGHTS.
///
/// A rule holds when its enable flag is TRUE, its location covers the point
/// and its time range is TRUE (Clause 12.34.9.1).
///
/// # Locations
///
/// - ALL covers every point, whatever the location reference holds.
/// - SPECIFIED with an Access Point covers that point only.
/// - SPECIFIED with an Access Zone covers the point when the zone's
///   Entry_Points names it.
/// - Anything else covers nothing: no reference, an unspecified one, one to
///   another device, a missing zone, another object type, or a specifier
///   outside the two named values.
///
/// A negative rule that holds denies with DENIED_POINT_NO_ACCESS_RIGHTS when
/// its location names the point and DENIED_ZONE_NO_ACCESS_RIGHTS when it
/// names a zone. Clause 12.34.9.2 names no value for a negative rule covering
/// every location; such a rule bars passage through this point as much as
/// one naming it, so this evaluator reports DENIED_POINT_NO_ACCESS_RIGHTS.
///
/// # Time ranges
///
/// ALWAYS is TRUE. SPECIFIED reads the referenced property from this
/// database, honouring the reference's array index, and turns the value into
/// TRUE or FALSE:
///
/// - BOOLEAN: its value.
/// - Unsigned: TRUE unless zero.
/// - INTEGER: TRUE above zero.
/// - Enumerated: TRUE for ACTIVE (1), FALSE for INACTIVE (0). A read value
///   doesn't say which enumeration it belongs to, so every Enumerated is
///   read as a BACnetBinaryPV, and any other number is FALSE. Clause
///   12.34.9.1 leaves types other than the four it names to the device, so
///   this is a local choice.
/// - Any other type, REAL and CharacterString among them: FALSE, the same
///   local choice.
///
/// The time range is FALSE when the reference is missing or unspecified,
/// names another device, or names a missing object, a property the object
/// doesn't serve, an index on a property that isn't an array, or one past
/// the end; when the read fails; and when the value is NULL. A specifier
/// outside the two named values is FALSE too.
///
/// # Which device a reference names
///
/// A reference names this device when it has no device member, when its
/// device member is the wildcard Device instance 4194303, or when it names
/// the Device the database speaks for ([`ObjectDatabase::local_device`]).
/// Anything else names another device, and this evaluator reads nothing
/// remotely: such a time range or location is FALSE and such an
/// Assigned_Access_Rights element is listed as unresolved. A database
/// holding no Device knows no Device of its own, so then only the first two
/// forms are local.
///
/// # Not covered
///
/// The Access Rights check is one of several authorization checks. This
/// evaluator doesn't judge Accompaniment (Clause 12.34.11), the credential's
/// status, validity window or threat authority, the point's authorization
/// mode, occupancy or passback, and it doesn't record the outcome on the
/// point.
///
/// # Errors
///
/// OBJECT / UNKNOWN_OBJECT, before any rule is read, when
/// `credential` names no Access Credential in `db` or `point` no Access
/// Point. An error reading or decoding the credential's
/// Assigned_Access_Rights is passed on; the built-in credential always
/// serves it whole.
///
/// # Example
///
/// ```
/// use bacnet_objects::access_control::{
///     evaluate_access_rights, AccessCredentialObject, AccessPointObject, AccessRightsObject,
/// };
/// use bacnet_objects::database::ObjectDatabase;
/// use bacnet_types::constructed::{BACnetAccessRule, BACnetAssignedAccessRights};
/// use bacnet_types::enums::{AccessEvent, ObjectType};
/// use bacnet_types::primitives::ObjectIdentifier;
///
/// let door = ObjectIdentifier::new(ObjectType::ACCESS_POINT, 1)?;
/// let staff = ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, 1)?;
/// let card = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1)?;
///
/// // Staff may pass Point 1 at any time.
/// let mut rights = AccessRightsObject::new(1, "STAFF")?;
/// rights.set_positive_access_rules([BACnetAccessRule::new(None, Some(door.into()), true)])?;
/// let mut credential = AccessCredentialObject::new(1, "CARD-1")?;
/// credential.set_assigned_access_rights(vec![BACnetAssignedAccessRights {
///     assigned_access_rights: staff.into(),
///     enable: true,
/// }])?;
///
/// let mut db = ObjectDatabase::new();
/// db.add(Box::new(AccessPointObject::new(1, "DOOR-1")?))?;
/// db.add(Box::new(AccessPointObject::new(2, "DOOR-2")?))?;
/// db.add(Box::new(rights))?;
/// db.add(Box::new(credential))?;
///
/// assert!(evaluate_access_rights(&db, card, door)?.decision.allows());
/// let elsewhere = ObjectIdentifier::new(ObjectType::ACCESS_POINT, 2)?;
/// let evaluation = evaluate_access_rights(&db, card, elsewhere)?;
/// assert_eq!(
///     evaluation.decision.access_event(),
///     Some(AccessEvent::DENIED_NO_ACCESS_RIGHTS)
/// );
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
pub fn evaluate_access_rights(
    db: &ObjectDatabase,
    credential: ObjectIdentifier,
    point: ObjectIdentifier,
) -> Result<AccessRightsEvaluation, Error> {
    let holder = object_of_type(db, credential, ObjectType::ACCESS_CREDENTIAL)?;
    object_of_type(db, point, ObjectType::ACCESS_POINT)?;
    if exempt(holder) {
        return Ok(AccessRightsEvaluation {
            decision: AccessRightsDecision::Exempt,
            unresolved: Vec::new(),
        });
    }
    let scope = Scope {
        db,
        local: db.local_device(),
        point,
    };
    let (gathered, unresolved) = scope.gather(&assigned_access_rights(holder)?);
    Ok(AccessRightsEvaluation {
        decision: scope.decide(&gathered),
        unresolved,
    })
}

/// The object `oid` names in `db`, when it is of `object_type`.
fn object_of_type(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    object_type: ObjectType,
) -> Result<&dyn BACnetObject, Error> {
    db.get(&oid)
        .filter(|_| oid.object_type() == object_type)
        .ok_or_else(|| common::protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
}

/// Whether the credential serves an Authorization_Exemptions list holding
/// ACCESS_RIGHTS.
fn exempt(credential: &dyn BACnetObject) -> bool {
    let access_rights = PropertyValue::Enumerated(AuthorizationExemption::ACCESS_RIGHTS.to_raw());
    matches!(
        credential.read_property(PropertyIdentifier::AUTHORIZATION_EXEMPTIONS, None),
        Ok(PropertyValue::List(exemptions)) if exemptions.contains(&access_rights)
    )
}

/// The credential's Assigned_Access_Rights, decoded from the value it serves.
fn assigned_access_rights(
    credential: &dyn BACnetObject,
) -> Result<Vec<BACnetAssignedAccessRights>, Error> {
    let value = credential.read_property(PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS, None)?;
    // Each element opens with the reference's frame, context tag [0].
    common::decode_elements(
        &value,
        |tag| tag.is_opening_tag(0),
        decode_assigned_access_rights,
    )
}

/// The enabled rules of one gathered Access Rights object.
struct Gathered {
    oid: ObjectIdentifier,
    negative: Vec<BACnetAccessRule>,
    positive: Vec<BACnetAccessRule>,
}

impl Gathered {
    fn rules(&self, kind: AccessRuleKind) -> &[BACnetAccessRule] {
        match kind {
            AccessRuleKind::Positive => &self.positive,
            AccessRuleKind::Negative => &self.negative,
        }
    }
}

/// Every enabled rule of `kind`, objects in gathering order and rules in
/// array order, with its position.
fn enabled_rules(
    gathered: &[Gathered],
    kind: AccessRuleKind,
) -> impl Iterator<Item = (AccessRulePosition, &BACnetAccessRule)> {
    gathered.iter().flat_map(move |rights| {
        (1..)
            .zip(rights.rules(kind))
            .filter(|(_, rule)| rule.enable)
            .map(move |(index, rule)| {
                let position = AccessRulePosition {
                    access_rights: rights.oid,
                    kind,
                    index,
                };
                (position, rule)
            })
    })
}

/// What a rule's location matched: the kind of place decides the
/// Access_Event a negative rule denies with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Location-Specifier ALL.
    Everywhere,
    /// The Access Point itself.
    Point,
    /// An Access Zone the point enters.
    Zone,
}

impl Place {
    /// The Access_Event of a negative rule that held here (Clause
    /// 12.34.9.2). A rule for every location bars this point as well, so it
    /// is reported as the point's denial: a local reading, since the clause
    /// names only the point and zone cases.
    fn denial(self) -> AccessEvent {
        match self {
            Place::Zone => AccessEvent::DENIED_ZONE_NO_ACCESS_RIGHTS,
            Place::Point | Place::Everywhere => AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS,
        }
    }
}

/// One evaluation: the database, its own Device and the point where the
/// credential was presented.
#[derive(Clone, Copy)]
struct Scope<'a> {
    db: &'a ObjectDatabase,
    local: LocalDevice,
    point: ObjectIdentifier,
}

impl Scope<'_> {
    /// Whether a reference with this device member names this device: no
    /// member, the wildcard Device, or the database's own Device.
    fn is_local(&self, device: Option<ObjectIdentifier>) -> bool {
        device.is_none_or(|device| {
            let wildcard = device.object_type() == ObjectType::DEVICE
                && device.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE;
            wildcard || self.local.is_local(Some(device))
        })
    }

    /// The enabled Access Rights objects the enabled elements of
    /// `assigned` name, in order, and the elements that gave no rules.
    fn gather(
        &self,
        assigned: &[BACnetAssignedAccessRights],
    ) -> (Vec<Gathered>, Vec<UnresolvedAccessRights>) {
        let mut gathered = Vec::new();
        let mut unresolved = Vec::new();
        for (index, element) in (1..).zip(assigned) {
            let reference = &element.assigned_access_rights;
            if !element.enable
                || unspecified(reference.object_identifier, reference.device_identifier)
            {
                continue;
            }
            match self.resolve(reference) {
                Ok(Some(rights)) => gathered.push(rights),
                // An Access Rights object with Enable FALSE.
                Ok(None) => {}
                Err(reason) => unresolved.push(UnresolvedAccessRights {
                    index,
                    reference: reference.clone(),
                    reason,
                }),
            }
        }
        (gathered, unresolved)
    }

    /// The rules of the Access Rights object `reference` names: `None`
    /// while its Enable is FALSE, or why it gives none.
    fn resolve(
        &self,
        reference: &BACnetDeviceObjectReference,
    ) -> Result<Option<Gathered>, UnresolvedReason> {
        let oid = reference.object_identifier;
        if !self.is_local(reference.device_identifier) {
            return Err(UnresolvedReason::Remote);
        }
        if oid.object_type() != ObjectType::ACCESS_RIGHTS {
            return Err(UnresolvedReason::NotAccessRights);
        }
        let object = self.db.get(&oid).ok_or(UnresolvedReason::Missing)?;
        // Table 12-39's Enable is property 133, LOG_ENABLE in the enum.
        match object.read_property(PropertyIdentifier::LOG_ENABLE, None) {
            Ok(PropertyValue::Boolean(true)) => {}
            Ok(PropertyValue::Boolean(false)) => return Ok(None),
            _ => return Err(UnresolvedReason::Unreadable),
        }
        let rules = |kind: AccessRuleKind| {
            object
                .read_property(kind.property(), None)
                .and_then(|value| {
                    // A rule opens with its time-range specifier, context tag [0].
                    common::decode_elements(&value, |tag| tag.is_context(0), decode_access_rule)
                })
                .map_err(|_| UnresolvedReason::Unreadable)
        };
        Ok(Some(Gathered {
            oid,
            negative: rules(AccessRuleKind::Negative)?,
            positive: rules(AccessRuleKind::Positive)?,
        }))
    }

    /// The decision over the gathered rules (Clause 12.34.9.2).
    fn decide(&self, gathered: &[Gathered]) -> AccessRightsDecision {
        for (position, rule) in enabled_rules(gathered, AccessRuleKind::Negative) {
            if let Some(place) = self.location(rule) {
                if self.time_range(rule) {
                    return AccessRightsDecision::Denied {
                        access_event: place.denial(),
                        rule: Some(position),
                    };
                }
            }
        }
        let mut out_of_time = None;
        for (position, rule) in enabled_rules(gathered, AccessRuleKind::Positive) {
            if self.location(rule).is_some() {
                if self.time_range(rule) {
                    return AccessRightsDecision::Granted { rule: position };
                }
                out_of_time.get_or_insert(position);
            }
        }
        match out_of_time {
            Some(position) => AccessRightsDecision::Denied {
                access_event: AccessEvent::DENIED_OUT_OF_TIME_RANGE,
                rule: Some(position),
            },
            None => AccessRightsDecision::Denied {
                access_event: AccessEvent::DENIED_NO_ACCESS_RIGHTS,
                rule: None,
            },
        }
    }

    /// What the rule's location matched at this point, if anything.
    fn location(&self, rule: &BACnetAccessRule) -> Option<Place> {
        match rule.location_specifier {
            AccessRuleLocationSpecifier::ALL => Some(Place::Everywhere),
            AccessRuleLocationSpecifier::SPECIFIED => {
                let reference = rule.location.as_ref()?;
                let oid = reference.object_identifier;
                if unspecified(oid, reference.device_identifier)
                    || !self.is_local(reference.device_identifier)
                {
                    return None;
                }
                match oid.object_type() {
                    ObjectType::ACCESS_POINT => (oid == self.point).then_some(Place::Point),
                    ObjectType::ACCESS_ZONE => self.enters(oid).then_some(Place::Zone),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether the zone `zone` names the point among its Entry_Points.
    fn enters(&self, zone: ObjectIdentifier) -> bool {
        let Some(zone) = self.db.get(&zone) else {
            return false;
        };
        let Ok(value) = zone.read_property(PropertyIdentifier::ENTRY_POINTS, None) else {
            return false;
        };
        decode_references::<BACnetDeviceObjectReference>(&value).is_ok_and(|entries| {
            entries.iter().any(|entry| {
                entry.object_identifier == self.point && self.is_local(entry.device_identifier)
            })
        })
    }

    /// The rule's time range: TRUE or FALSE (Clause 12.34.9.1).
    fn time_range(&self, rule: &BACnetAccessRule) -> bool {
        match rule.time_range_specifier {
            AccessRuleTimeRangeSpecifier::ALWAYS => true,
            AccessRuleTimeRangeSpecifier::SPECIFIED => rule
                .time_range
                .as_ref()
                .is_some_and(|reference| self.reads_true(reference)),
            _ => false,
        }
    }

    /// Whether the property `reference` names reads as TRUE here.
    fn reads_true(&self, reference: &BACnetDeviceObjectPropertyReference) -> bool {
        let oid = reference.object_identifier;
        if unspecified(oid, reference.device_identifier)
            || !self.is_local(reference.device_identifier)
        {
            return false;
        }
        let Some(object) = self.db.get(&oid) else {
            return false;
        };
        let property = PropertyIdentifier::from_raw(reference.property_identifier);
        let index = reference.property_array_index;
        // ReadProperty's gate: only an array takes an index.
        if index.is_some() && !object.is_array_property(property) {
            return false;
        }
        object
            .read_property(property, index)
            .is_ok_and(|value| truth(&value))
    }
}

/// A time-range value as TRUE or FALSE (Clause 12.34.9.1, and the local
/// choices in the module documentation).
fn truth(value: &PropertyValue) -> bool {
    match *value {
        PropertyValue::Boolean(value) => value,
        PropertyValue::Unsigned(value) => value != 0,
        PropertyValue::Signed(value) => value > 0,
        PropertyValue::Enumerated(raw) => raw == BinaryPV::ACTIVE.to_raw(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "rights_evaluation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rights_evaluation_term_tests.rs"]
mod term_tests;

#[cfg(test)]
#[path = "rights_evaluation_scenario_tests.rs"]
mod scenario_tests;
