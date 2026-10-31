//! ReadProperty, and the single-property reads ReadRange and a host's
//! local read share with it: plan the read, then read the plan.
use super::group::{self, GroupMembers};
use super::rpm::{self, PlannedRow};
use super::{LiveLists, ReadFailure, ReadOverlay, ReadScope};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

/// One single-property read (ReadProperty, ReadRange or the local read),
/// planned before any value is read: its target and row, with the member rows
/// of a Group's whole Present_Value charged to the scope's work limit (#1172).
/// Planning reads no value, so the request chooses its COV snapshot from the
/// plan and reads afterwards (#1213).
pub struct PropertyPlan {
    lookup_oid: ObjectIdentifier,
    row: PlannedRow,
}

impl PropertyPlan {
    /// Plan the read of `property` on `stored`, the object the service's own
    /// lookup found at `lookup_oid`. A missing object plans nothing more; the
    /// read reports it.
    pub fn new<O: ReadOverlay>(
        db: &ObjectDatabase,
        scope: &ReadScope,
        overlay: Option<&O>,
        lookup_oid: ObjectIdentifier,
        stored: Option<&dyn BACnetObject>,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<Self, ReadFailure> {
        let reference = PropertyReference {
            property_identifier: property,
            property_array_index: array_index,
        };
        let members = match stored {
            Some(stored) => {
                let served = overlay.and_then(|overlay| overlay.view(stored));
                let object: &dyn BACnetObject = served
                    .as_ref()
                    .map_or(stored, |served| served as &dyn BACnetObject);
                group::plan_property(db, scope, overlay, object, &reference)?
            }
            None => None,
        };
        Ok(Self {
            lookup_oid,
            row: PlannedRow { reference, members },
        })
    }

    /// The selected Device's COV lists this read takes, by name or through
    /// the planned members of a Group.
    pub fn live_lists(&self, db: &ObjectDatabase) -> Option<LiveLists> {
        rpm::live_cov_selection(db, [(self.lookup_oid, &self.row)])
    }

    /// The planned members of a Group whose whole Present_Value the plan
    /// reads, for a host that reads the row itself (ReadRange).
    pub fn into_members(self) -> Option<GroupMembers> {
        self.row.members
    }
}

/// Plan a ReadProperty, or a local read, of the resolved `lookup_oid`. Past
/// the scope's work limit it fails with [`ReadFailure::Work`], before any
/// value is read.
pub fn plan_property<O: ReadOverlay>(
    db: &ObjectDatabase,
    scope: &ReadScope,
    overlay: Option<&O>,
    lookup_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<PropertyPlan, ReadFailure> {
    PropertyPlan::new(
        db,
        scope,
        overlay,
        lookup_oid,
        read_target_object(db, &lookup_oid),
        property,
        array_index,
    )
}

/// ReadProperty evaluator over one decoded request and its plan. `overlay`
/// carries executor-owned Device definitions and the request-local COV lists
/// the plan selected; observations carry only execution outcomes, never the
/// read value. A read past the work limit failed in planning and is not
/// observed, as a ReadPropertyMultiple over its work budget is not.
pub fn read_property_ack<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    request: &ReadPropertyRequest,
    plan: PropertyPlan,
    buf: &mut BytesMut,
    mut completed: impl FnMut(ObjectIdentifier, &ReadPropertyRequest, &Result<(), Error>),
) -> Result<(), Error> {
    let lookup_oid = plan.lookup_oid;
    let result = read_property_decoded(db, overlay, request, plan, buf);
    completed(lookup_oid, request, &result);
    result
}

/// Evaluate one planned property read with ReadProperty error precedence:
/// unknown object, then non-array index using the effective property
/// definition, then the overlay's view or raw object's reader, except that a
/// Group's Present_Value is rebuilt from the members its plan holds.
pub fn read_value<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    plan: PropertyPlan,
) -> Result<PropertyValue, Error> {
    let PropertyPlan {
        lookup_oid,
        row: PlannedRow { reference, members },
    } = plan;
    let property = reference.property_identifier;
    let array_index = reference.property_array_index;
    let object = read_target_object(db, &lookup_oid).ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))?;

    let served = overlay.and_then(|overlay| overlay.view(object));
    let object: &dyn BACnetObject = served
        .as_ref()
        .map_or(object, |served| served as &dyn BACnetObject);

    // Clause 15.5.1.3: an array index on a non-array property is rejected
    // with PROPERTY / PROPERTY_IS_NOT_AN_ARRAY. The array/list decision
    // belongs to the object (identifier-static whitelists cannot express the
    // type-dependent identifiers, e.g. ALARM_VALUES), so the handler defers
    // to the trait query.
    if array_index.is_some() && !object.is_array_property(property) {
        return Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }

    group::read_served_property(db, overlay, object, property, array_index, members)
}

fn read_property_decoded<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    request: &ReadPropertyRequest,
    plan: PropertyPlan,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let lookup_oid = plan.lookup_oid;
    let value = read_value(db, overlay, plan)?;

    let mut value_buf = BytesMut::new();
    encode_property_value(&mut value_buf, &value)?;

    let ack = ReadPropertyACK {
        object_identifier: lookup_oid,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        property_value: value_buf.to_vec(),
    };

    ack.encode(buf);
    Ok(())
}

/// The stored object a read of `oid` targets: never one at the unregistered
/// Network_Port wildcard instance.
///
/// For bacnet-server's legacy, unbudgeted ReadPropertyMultiple helper only;
/// it leaves the public API with that helper (#2178, PR 3).
#[doc(hidden)]
pub fn read_target_object<'a>(
    db: &'a ObjectDatabase,
    oid: &ObjectIdentifier,
) -> Option<&'a dyn bacnet_objects::traits::BACnetObject> {
    // An unregistered wildcard is never an ordinary stored object, even if a
    // custom database supplied that reserved identifier.
    if oid.object_type() == ObjectType::NETWORK_PORT && oid.instance_number() == 4194303 {
        return None;
    }
    db.get(oid)
}

fn is_device_wildcard(oid: &ObjectIdentifier) -> bool {
    oid.object_type() == ObjectType::DEVICE && oid.instance_number() == 4194303
}

/// Resolve Device or explicitly registered receiving-port wildcard identity.
pub fn resolve_target(
    db: &ObjectDatabase,
    scope: &ReadScope,
    oid: &ObjectIdentifier,
) -> ObjectIdentifier {
    if oid.object_type() == ObjectType::NETWORK_PORT && oid.instance_number() == 4194303 {
        return scope
            .registered_port
            .filter(|selected| Some(*selected) == db.registered_port_internal())
            .unwrap_or(*oid);
    }
    if is_device_wildcard(oid) {
        // A configured executor never rebinds to another Device after mutation.
        // Returning a missing frozen OID preserves UNKNOWN_OBJECT downstream.
        if let Some(device) = scope.device.or_else(|| db.selected_device()) {
            return device;
        }
    }
    *oid
}

/// Clause 15.7.3.2.2.2 includes an index only for a declared array property.
/// An unknown object/property or unavailable declaration conservatively omits
/// it. Classification must not probe `read_property` or change error precedence.
///
/// For bacnet-server's legacy, unbudgeted ReadPropertyMultiple helper only;
/// it leaves the public API with that helper (#2178, PR 3).
#[doc(hidden)]
pub fn rpm_response_index(
    object: Option<&dyn bacnet_objects::traits::BACnetObject>,
    property: PropertyIdentifier,
    requested: Option<u32>,
) -> Option<u32> {
    let index = requested?;
    let object = object?;
    let present = object
        .property_metadata()
        .iter()
        .any(|row| row.property_identifier == property);
    (present && object.is_array_property(property)).then_some(index)
}
