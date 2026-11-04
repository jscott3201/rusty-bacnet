//! Exact readback snapshots for server-owned Life Safety COV mutation paths.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use bacnet_objects::database::ObjectDatabase;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::CovSubscription;

/// The COV surface a Point and a Zone share, Tracking_Value included.
const PROPERTIES: [PropertyIdentifier; 5] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::TRACKING_VALUE,
    PropertyIdentifier::SILENCED,
    PropertyIdentifier::OPERATION_EXPECTED,
    PropertyIdentifier::STATUS_FLAGS,
];

/// The COV properties of one Life Safety object a mutation changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifeSafetyCovChange {
    /// The Point or Zone.
    pub object_identifier: ObjectIdentifier,
    /// Its COV properties whose readback differs from before the mutation.
    pub changed_properties: Vec<PropertyIdentifier>,
}

impl LifeSafetyCovChange {
    /// Whether Status_Flags changed: every subscription to the object then
    /// reports, whatever its baseline.
    pub fn status_changed(&self) -> bool {
        self.changed_properties
            .contains(&PropertyIdentifier::STATUS_FLAGS)
    }

    /// Whether this change notifies `subscription` to the object: a
    /// whole-object one on a Present_Value change, a property one on a
    /// change of its property, and either on a Status_Flags change.
    pub fn notifies(&self, subscription: &CovSubscription) -> bool {
        self.status_changed()
            || self.changed_properties.contains(
                &subscription
                    .monitored_property
                    .unwrap_or(PropertyIdentifier::PRESENT_VALUE),
            )
    }

    /// A change of `changed_properties`, or `None` when there are none.
    pub fn new(
        object_identifier: ObjectIdentifier,
        changed_properties: Vec<PropertyIdentifier>,
    ) -> Option<Self> {
        (!changed_properties.is_empty()).then_some(Self {
            object_identifier,
            changed_properties,
        })
    }
}

#[derive(Debug, Clone)]
struct ObjectSnapshot {
    object_identifier: ObjectIdentifier,
    values: Vec<(PropertyIdentifier, PropertyValue)>,
}

/// Pre-mutation readback for the exact Life Safety COV surface.
#[derive(Debug, Clone, Default)]
pub struct LifeSafetyCovSnapshots {
    objects: Vec<ObjectSnapshot>,
    held: BTreeSet<ObjectIdentifier>,
}

/// Whether `object_identifier` names a Life Safety Point or Zone.
pub fn is_life_safety_object(object_identifier: ObjectIdentifier) -> bool {
    matches!(
        object_identifier.object_type(),
        ObjectType::LIFE_SAFETY_POINT | ObjectType::LIFE_SAFETY_ZONE
    )
}

fn properties_for(object_identifier: ObjectIdentifier) -> Option<&'static [PropertyIdentifier]> {
    is_life_safety_object(object_identifier).then_some(&PROPERTIES)
}

impl LifeSafetyCovSnapshots {
    /// Whether `object_identifier`'s pre-mutation state is held.
    pub fn holds(&self, object_identifier: ObjectIdentifier) -> bool {
        self.held.contains(&object_identifier)
    }

    /// Capture one object's pre-mutation state.
    pub fn capture_oid(db: &ObjectDatabase, object_identifier: ObjectIdentifier) -> Self {
        Self::capture_oids(db, [object_identifier])
    }

    /// Capture the object a WriteProperty request names, if it decodes.
    pub fn capture_write_property(db: &ObjectDatabase, service_data: &[u8]) -> Self {
        WritePropertyRequest::decode(service_data)
            .map(|request| Self::capture_oid(db, request.object_identifier))
            .unwrap_or_default()
    }

    /// Capture a Life Safety object immediately before its first WPM attempt.
    pub fn capture_before_write(
        &mut self,
        db: &ObjectDatabase,
        object_identifier: ObjectIdentifier,
    ) {
        if self
            .objects
            .iter()
            .any(|snapshot| snapshot.object_identifier == object_identifier)
        {
            return;
        }
        if let Some(snapshot) = Self::capture_oid(db, object_identifier).objects.pop() {
            self.objects.push(snapshot);
        }
    }

    /// The changes of the held objects among `affected_oids`, read back
    /// from `db` after the mutation.
    pub fn changes(
        &self,
        db: &ObjectDatabase,
        affected_oids: &[ObjectIdentifier],
    ) -> Vec<LifeSafetyCovChange> {
        self.objects
            .iter()
            .filter(|snapshot| affected_oids.contains(&snapshot.object_identifier))
            .filter_map(|snapshot| {
                let object = db.get(&snapshot.object_identifier)?;
                let changed_properties = snapshot
                    .values
                    .iter()
                    .filter_map(|(property, previous)| {
                        object
                            .read_property(*property, None)
                            .is_ok_and(|current| current != *previous)
                            .then_some(*property)
                    })
                    .collect();
                LifeSafetyCovChange::new(snapshot.object_identifier, changed_properties)
            })
            .collect()
    }

    /// Capture each object's pre-mutation state.
    pub fn capture_oids(
        db: &ObjectDatabase,
        object_identifiers: impl IntoIterator<Item = ObjectIdentifier>,
    ) -> Self {
        let mut snapshots = Self::default();
        for object_identifier in object_identifiers {
            snapshots.capture(db, object_identifier);
        }
        snapshots
    }

    /// Add one object's pre-mutation state unless it is already held or is not
    /// a Life Safety object.
    pub fn capture(&mut self, db: &ObjectDatabase, object_identifier: ObjectIdentifier) {
        let Some(properties) = properties_for(object_identifier) else {
            return;
        };
        let Some(object) = db.get(&object_identifier) else {
            return;
        };
        if !self.held.insert(object_identifier) {
            return;
        }
        let values = properties
            .iter()
            .filter_map(|property| {
                object
                    .read_property(*property, None)
                    .ok()
                    .map(|value| (*property, value))
            })
            .collect();
        self.objects.push(ObjectSnapshot {
            object_identifier,
            values,
        });
    }
}
