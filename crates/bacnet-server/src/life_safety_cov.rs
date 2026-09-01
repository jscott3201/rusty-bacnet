//! Exact readback snapshots for server-owned Life Safety COV mutation paths.

use bacnet_objects::database::ObjectDatabase;
use bacnet_services::wpm::WritePropertyMultipleRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

const POINT_PROPERTIES: [PropertyIdentifier; 5] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::TRACKING_VALUE,
    PropertyIdentifier::SILENCED,
    PropertyIdentifier::OPERATION_EXPECTED,
    PropertyIdentifier::STATUS_FLAGS,
];

const ZONE_PROPERTIES: [PropertyIdentifier; 4] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::SILENCED,
    PropertyIdentifier::OPERATION_EXPECTED,
    PropertyIdentifier::STATUS_FLAGS,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifeSafetyCovChange {
    pub(crate) object_identifier: ObjectIdentifier,
    pub(crate) changed_properties: Vec<PropertyIdentifier>,
}

impl LifeSafetyCovChange {
    pub(crate) fn new(
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
pub(crate) struct LifeSafetyCovSnapshots {
    objects: Vec<ObjectSnapshot>,
}

pub(crate) fn is_life_safety_object(object_identifier: ObjectIdentifier) -> bool {
    matches!(
        object_identifier.object_type(),
        ObjectType::LIFE_SAFETY_POINT | ObjectType::LIFE_SAFETY_ZONE
    )
}

fn properties_for(object_identifier: ObjectIdentifier) -> Option<&'static [PropertyIdentifier]> {
    match object_identifier.object_type() {
        ObjectType::LIFE_SAFETY_POINT => Some(&POINT_PROPERTIES),
        ObjectType::LIFE_SAFETY_ZONE => Some(&ZONE_PROPERTIES),
        _ => None,
    }
}

impl LifeSafetyCovSnapshots {
    pub(crate) fn capture_oid(db: &ObjectDatabase, object_identifier: ObjectIdentifier) -> Self {
        Self::capture_oids(db, [object_identifier])
    }

    pub(crate) fn capture_write_property(db: &ObjectDatabase, service_data: &[u8]) -> Self {
        WritePropertyRequest::decode(service_data)
            .map(|request| Self::capture_oid(db, request.object_identifier))
            .unwrap_or_default()
    }

    pub(crate) fn capture_write_property_multiple(
        db: &ObjectDatabase,
        service_data: &[u8],
    ) -> Self {
        let Ok(request) = WritePropertyMultipleRequest::decode(service_data) else {
            return Self::default();
        };
        Self::capture_oids(
            db,
            request
                .list_of_write_access_specs
                .iter()
                .map(|spec| spec.object_identifier),
        )
    }

    pub(crate) fn changes(
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

    pub(crate) fn capture_oids(
        db: &ObjectDatabase,
        object_identifiers: impl IntoIterator<Item = ObjectIdentifier>,
    ) -> Self {
        let mut objects = Vec::new();
        for object_identifier in object_identifiers {
            if objects
                .iter()
                .any(|snapshot: &ObjectSnapshot| snapshot.object_identifier == object_identifier)
            {
                continue;
            }
            let Some(properties) = properties_for(object_identifier) else {
                continue;
            };
            let Some(object) = db.get(&object_identifier) else {
                continue;
            };
            let values = properties
                .iter()
                .filter_map(|property| {
                    object
                        .read_property(*property, None)
                        .ok()
                        .map(|value| (*property, value))
                })
                .collect();
            objects.push(ObjectSnapshot {
                object_identifier,
                values,
            });
        }
        Self { objects }
    }
}
