//! The PICS object-type rows: one per type in the database and one per type
//! CreateObject builds (#2172, #2173).

use super::*;

impl PicsGenerator<'_> {
    pub(super) fn build_object_types(&self) -> Vec<ObjectTypeSupport> {
        let constructors = &self.server_config.object_constructors;
        let mut by_type: BTreeMap<u32, Vec<&dyn BACnetObject>> = BTreeMap::new();
        for (_oid, obj) in self.db.iter_objects() {
            by_type
                .entry(obj.object_identifier().object_type().to_raw())
                .or_default()
                .push(obj);
        }
        // Createability belongs to the type, so a createable type is listed
        // before its first instance exists.
        for kind in constructors
            .object_types()
            .chain(bacnet_objects::creation::builtin_createable_types())
        {
            by_type.entry(kind.to_raw()).or_default();
        }

        let mut result = Vec::with_capacity(by_type.len());
        for (raw_type, objects) in &by_type {
            let object_type = ObjectType::from_raw(*raw_type);
            let createable = constructors.creates(object_type);
            // With no instance, a built-in type is described from an object
            // its table constructor builds, outside the database. A
            // registered type gets a type-only row instead: the PICS never
            // calls the application's constructor.
            let built = (objects.is_empty() && !constructors.contains(object_type))
                .then(|| bacnet_objects::creation::builtin_constructor(object_type))
                .flatten()
                .and_then(|constructor| constructor(1, format!("{object_type}-1"), &[]).ok());
            let objects: Vec<&dyn BACnetObject> = match &built {
                Some(object) => vec![object.as_ref()],
                None => objects.clone(),
            };
            let Some(&representative) = objects.first() else {
                result.push(ObjectTypeSupport {
                    object_type,
                    createable,
                    creation_only_properties: Vec::new(),
                    deleteable: true,
                    supported_properties: Vec::new(),
                });
                continue;
            };
            let view = (self.served && object_type == ObjectType::DEVICE).then(|| {
                crate::device_view::DeviceReadContext::new(
                    self.db,
                    crate::device_view::DeviceExecution::FullServer,
                )
                .with_identity(Some(&self.server_config.device))
            });
            let supported_properties =
                Self::union_property_support(objects.iter().flat_map(|object| {
                    if let Some(view) = &view {
                        Self::object_property_support(&view.object(*object))
                    } else {
                        Self::object_property_support(*object)
                    }
                }));

            // Deletion is asked of the instances' representative; creation
            // is the type-level answer CreateObject itself follows.
            let deleteable = representative.is_deleteable();
            let written_through = |property: &PropertyIdentifier| {
                supported_properties
                    .iter()
                    .any(|row| row.property_id == *property && row.written_through.is_some())
            };
            let creation_only_properties = if createable {
                let mut properties = representative.creation_only_properties().to_vec();
                properties.retain(|property| !written_through(property));
                properties
            } else {
                Vec::new()
            };

            result.push(ObjectTypeSupport {
                object_type,
                createable,
                creation_only_properties,
                deleteable,
                supported_properties,
            });
        }
        result
    }
}

#[cfg(test)]
impl Pics {
    /// Only the rows of types with an instance in `db`: what a test about
    /// the rows its own objects produce compares, without the rows every
    /// createable type has before its first instance.
    pub(crate) fn instance_types_only(mut self, db: &ObjectDatabase) -> Self {
        self.supported_object_types
            .retain(|support| !db.find_by_type(support.object_type).is_empty());
        self
    }
}
