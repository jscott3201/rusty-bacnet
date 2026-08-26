//! ObjectDatabase — stores and retrieves BACnet objects by identifier.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::clock::{ClockFrame, ClockReader};
use crate::event_enrollment::EventEnrollmentMonitoredSource;
use crate::traits::BACnetObject;

/// A collection of BACnet objects, keyed by ObjectIdentifier.
///
/// Enforces Object_Name uniqueness within a device.
/// Maintains secondary indexes for O(1) name lookup and O(1) type lookup.
pub struct ObjectDatabase {
    objects: HashMap<ObjectIdentifier, Box<dyn BACnetObject>>,
    /// Shared Device clock reader. `None` is an explicit clockless database.
    clock: Option<Arc<dyn ClockReader>>,
    /// Device-local EventNotification ordering source for clockless operation.
    event_sequence_number: u16,
    /// Reverse index: object name → ObjectIdentifier for uniqueness enforcement.
    name_index: HashMap<String, ObjectIdentifier>,
    /// Type index: object type → set of ObjectIdentifiers for fast enumeration.
    type_index: HashMap<ObjectType, Vec<ObjectIdentifier>>,
    /// Event Enrollment objects whose private evaluator state must be reset
    /// before it can be used again.
    invalid_enrollment_eval_state: HashSet<ObjectIdentifier>,
    /// Source ownership for custom Event Enrollment objects that implement
    /// evaluation state but not the optional object-owned source channel.
    enrollment_eval_sources: HashMap<ObjectIdentifier, EventEnrollmentMonitoredSource>,
}

impl Default for ObjectDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectDatabase {
    /// Create an empty database.
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            clock: None,
            event_sequence_number: 0,
            name_index: HashMap::new(),
            type_index: HashMap::new(),
            invalid_enrollment_eval_state: HashSet::new(),
            enrollment_eval_sources: HashMap::new(),
        }
    }

    /// Add an object to the database.
    ///
    /// Returns `Err` if another object already has the same `object_name()`.
    /// Replacing an object with the same OID is allowed (the old object is removed).
    pub fn add(&mut self, mut object: Box<dyn BACnetObject>) -> Result<(), Error> {
        object.bind_clock_internal(self.clock.clone());
        let oid = object.object_identifier();
        let name = object.object_name().to_string();

        // Check for name collision with a *different* object
        if let Some(&existing_oid) = self.name_index.get(&name) {
            if existing_oid != oid {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::DUPLICATE_NAME.to_raw() as u32,
                });
            }
        }

        // If replacing an existing object, remove its old name from the index
        // and invalidate state owned by enrollments that monitor it.
        if let Some(old) = self.objects.get(&oid) {
            let old_name = old.object_name().to_string();
            self.name_index.remove(&old_name);
            self.invalidate_enrollments_monitoring(&oid);
        }

        self.name_index.insert(name, oid);
        let is_new = !self.objects.contains_key(&oid);
        self.enrollment_eval_sources.remove(&oid);
        self.objects.insert(oid, object);
        if is_new {
            self.type_index
                .entry(oid.object_type())
                .or_default()
                .push(oid);
        }
        Ok(())
    }

    /// Look up an object by its name. O(1) via the name index.
    pub fn find_by_name(&self, name: &str) -> Option<&dyn BACnetObject> {
        let oid = self.name_index.get(name)?;
        self.objects.get(oid).map(|o| o.as_ref())
    }

    /// Check whether `new_name` is available for object `oid`.
    ///
    /// Returns `Ok(())` if the name is unused or already belongs to `oid`.
    /// Returns `Err(DUPLICATE_NAME)` if another object owns the name.
    pub fn check_name_available(
        &self,
        oid: &ObjectIdentifier,
        new_name: &str,
    ) -> Result<(), Error> {
        if let Some(&owner) = self.name_index.get(new_name) {
            if owner != *oid {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::DUPLICATE_NAME.to_raw() as u32,
                });
            }
        }
        Ok(())
    }

    /// Update the name index after a successful Object_Name write.
    ///
    /// Call this after `write_property(OBJECT_NAME, …)` succeeds.
    pub fn update_name_index(&mut self, oid: &ObjectIdentifier) {
        if let Some(obj) = self.objects.get(oid) {
            // Remove any old name mapping for this OID
            self.name_index.retain(|_, v| v != oid);
            // Insert the current name
            self.name_index.insert(obj.object_name().to_string(), *oid);
        }
    }

    /// Get a shared reference to an object by identifier.
    pub fn get(&self, oid: &ObjectIdentifier) -> Option<&dyn BACnetObject> {
        self.objects.get(oid).map(|o| o.as_ref())
    }

    /// Get a mutable reference to an object by identifier.
    pub fn get_mut(&mut self, oid: &ObjectIdentifier) -> Option<&mut Box<dyn BACnetObject>> {
        self.objects.get_mut(oid)
    }

    /// Whether an Event Enrollment object's private evaluator state requires a
    /// successful reset before reuse.
    pub fn enrollment_eval_state_invalidated(&self, oid: &ObjectIdentifier) -> bool {
        self.invalid_enrollment_eval_state.contains(oid)
    }

    /// Mark or clear the reset requirement for Event Enrollment private state.
    pub fn set_enrollment_eval_state_invalidated(
        &mut self,
        oid: ObjectIdentifier,
        invalidated: bool,
    ) {
        if invalidated {
            self.invalid_enrollment_eval_state.insert(oid);
        } else {
            self.invalid_enrollment_eval_state.remove(&oid);
        }
    }

    /// Return database-owned monitored-source state for a custom Event
    /// Enrollment object.
    pub fn enrollment_eval_source(
        &self,
        oid: &ObjectIdentifier,
    ) -> Option<EventEnrollmentMonitoredSource> {
        self.enrollment_eval_sources.get(oid).copied()
    }

    /// Store database-owned monitored-source state for a custom Event
    /// Enrollment object.
    pub fn set_enrollment_eval_source(
        &mut self,
        oid: ObjectIdentifier,
        source: Option<EventEnrollmentMonitoredSource>,
    ) {
        if let Some(source) = source {
            self.enrollment_eval_sources.insert(oid, source);
        } else {
            self.enrollment_eval_sources.remove(&oid);
        }
    }

    fn invalidate_enrollments_monitoring(&mut self, monitored_oid: &ObjectIdentifier) {
        let affected = self
            .objects
            .iter()
            .filter_map(|(oid, object)| {
                let source = object
                    .enrollment_eval_source_internal()
                    .flatten()
                    .or_else(|| self.enrollment_eval_sources.get(oid).copied());
                source
                    .is_some_and(|source| source.0 == *monitored_oid)
                    .then_some(*oid)
            })
            .collect::<Vec<_>>();
        self.invalid_enrollment_eval_state.extend(affected);
    }

    /// Remove an object by identifier.
    pub fn remove(&mut self, oid: &ObjectIdentifier) -> Option<Box<dyn BACnetObject>> {
        if self.objects.contains_key(oid) {
            self.invalidate_enrollments_monitoring(oid);
        }
        if let Some(mut obj) = self.objects.remove(oid) {
            self.enrollment_eval_sources.remove(oid);
            if obj.enrollment_eval_state_internal().is_some() {
                if obj
                    .set_enrollment_eval_state_internal(Default::default())
                    .is_err()
                {
                    self.invalid_enrollment_eval_state.insert(*oid);
                } else {
                    self.invalid_enrollment_eval_state.remove(oid);
                }
                let _ = obj.set_enrollment_eval_source_internal(None);
            } else {
                self.invalid_enrollment_eval_state.remove(oid);
            }
            self.name_index.remove(obj.object_name());
            if let Some(type_set) = self.type_index.get_mut(&oid.object_type()) {
                type_set.retain(|o| o != oid);
            }
            Some(obj)
        } else {
            None
        }
    }

    /// List all object identifiers in the database.
    pub fn list_objects(&self) -> Vec<ObjectIdentifier> {
        self.objects.keys().copied().collect()
    }

    /// Find all objects of a given type.
    ///
    /// Returns a `Vec` of `ObjectIdentifier`s whose object type matches `object_type`.
    /// Useful for WhoHas, object enumeration, and similar queries.
    pub fn find_by_type(&self, object_type: ObjectType) -> Vec<ObjectIdentifier> {
        self.type_index
            .get(&object_type)
            .cloned()
            .unwrap_or_default()
    }

    /// Iterate over all `(ObjectIdentifier, &dyn BACnetObject)` pairs.
    ///
    /// Avoids the double-lookup pattern of `list_objects()` followed by `get()`.
    pub fn iter_objects(&self) -> impl Iterator<Item = (ObjectIdentifier, &dyn BACnetObject)> {
        self.objects.iter().map(|(&oid, obj)| (oid, obj.as_ref()))
    }

    /// Bind one clock reader to the database and every object it contains.
    ///
    /// Devices added after this call receive the same reader in [`add`](Self::add).
    pub fn set_clock_reader(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
        for object in self.objects.values_mut() {
            object.bind_clock_internal(self.clock.clone());
        }
    }

    /// Read one coherent sample from the database's shared clock.
    pub fn clock_frame(&self) -> Option<ClockFrame> {
        self.clock.as_ref()?.read_clock()
    }

    /// Consume the next Device-local EventNotification sequence number.
    ///
    /// The counter wraps modulo 65536 as required by the timestamp production.
    #[doc(hidden)]
    pub fn next_event_sequence_number(&mut self) -> u16 {
        let current = self.event_sequence_number;
        self.event_sequence_number = self.event_sequence_number.wrapping_add(1);
        current
    }

    /// Visit every `(ObjectIdentifier, &mut dyn BACnetObject)` pair.
    ///
    /// Mutable counterpart to [`iter_objects`](Self::iter_objects), used by
    /// the intrinsic-reporting tick task to advance pending transitions. A
    /// callback is used (rather than a returned `impl Iterator`) so the
    /// `&mut dyn BACnetObject` borrow is tied to this call's lifetime.
    pub fn for_each_object_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(ObjectIdentifier, &mut dyn BACnetObject),
    {
        for (&oid, obj) in self.objects.iter_mut() {
            f(oid, obj.as_mut());
        }
    }

    /// Number of objects in the database.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    #[test]
    fn event_sequence_wraps_and_is_database_local() {
        let mut first = ObjectDatabase::new();
        let mut second = ObjectDatabase::new();

        assert_eq!(first.next_event_sequence_number(), 0);
        assert_eq!(first.next_event_sequence_number(), 1);
        assert_eq!(second.next_event_sequence_number(), 0);

        first.event_sequence_number = u16::MAX;
        assert_eq!(first.next_event_sequence_number(), u16::MAX);
        assert_eq!(first.next_event_sequence_number(), 0);
        assert_eq!(second.next_event_sequence_number(), 1);
    }

    /// Minimal test object.
    struct TestObject {
        oid: ObjectIdentifier,
        name: String,
    }

    impl BACnetObject for TestObject {
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
            if property == PropertyIdentifier::OBJECT_NAME {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            } else {
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                })
            }
        }

        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            })
        }

        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            Cow::Borrowed(&[PropertyIdentifier::OBJECT_NAME])
        }
    }

    fn make_test_object(instance: u32) -> Box<dyn BACnetObject> {
        Box::new(TestObject {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            name: format!("AI-{instance}"),
        })
    }

    fn make_test_object_typed(
        object_type: ObjectType,
        instance: u32,
        name: &str,
    ) -> Box<dyn BACnetObject> {
        Box::new(TestObject {
            oid: ObjectIdentifier::new(object_type, instance).unwrap(),
            name: name.to_string(),
        })
    }

    #[test]
    fn add_and_get() {
        let mut db = ObjectDatabase::new();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        db.add(make_test_object(1)).unwrap();
        assert_eq!(db.len(), 1);

        let obj = db.get(&oid).unwrap();
        assert_eq!(obj.object_name(), "AI-1");
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let db = ObjectDatabase::new();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();
        assert!(db.get(&oid).is_none());
    }

    #[test]
    fn read_property_via_database() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object(1)).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let obj = db.get(&oid).unwrap();
        let val = obj
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("AI-1".into()));
    }

    #[test]
    fn remove_object() {
        let mut db = ObjectDatabase::new();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        db.add(make_test_object(1)).unwrap();
        assert_eq!(db.len(), 1);
        let removed = db.remove(&oid);
        assert!(removed.is_some());
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn list_objects() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object(1)).unwrap();
        db.add(make_test_object(2)).unwrap();
        let oids = db.list_objects();
        assert_eq!(oids.len(), 2);
    }

    #[test]
    fn find_by_type_returns_matching_objects() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "AI-1"))
            .unwrap();
        db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 2, "AI-2"))
            .unwrap();
        db.add(make_test_object_typed(ObjectType::BINARY_INPUT, 1, "BI-1"))
            .unwrap();
        db.add(make_test_object_typed(ObjectType::ANALOG_OUTPUT, 1, "AO-1"))
            .unwrap();

        let ai_oids = db.find_by_type(ObjectType::ANALOG_INPUT);
        assert_eq!(ai_oids.len(), 2);
        for oid in &ai_oids {
            assert_eq!(oid.object_type(), ObjectType::ANALOG_INPUT);
        }

        let bi_oids = db.find_by_type(ObjectType::BINARY_INPUT);
        assert_eq!(bi_oids.len(), 1);
        assert_eq!(bi_oids[0].object_type(), ObjectType::BINARY_INPUT);
        assert_eq!(bi_oids[0].instance_number(), 1);

        let ao_oids = db.find_by_type(ObjectType::ANALOG_OUTPUT);
        assert_eq!(ao_oids.len(), 1);
    }

    #[test]
    fn find_by_type_returns_empty_for_no_matches() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "AI-1"))
            .unwrap();

        let results = db.find_by_type(ObjectType::BINARY_VALUE);
        assert!(results.is_empty());
    }

    #[test]
    fn find_by_type_on_empty_database() {
        let db = ObjectDatabase::new();
        let results = db.find_by_type(ObjectType::ANALOG_INPUT);
        assert!(results.is_empty());
    }

    #[test]
    fn iter_objects_yields_all_entries() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "AI-1"))
            .unwrap();
        db.add(make_test_object_typed(ObjectType::BINARY_INPUT, 1, "BI-1"))
            .unwrap();

        let items: Vec<_> = db.iter_objects().collect();
        assert_eq!(items.len(), 2);

        // Verify we can access object data without a second lookup
        for (oid, obj) in &items {
            assert_eq!(oid.object_type(), obj.object_identifier().object_type());
            assert!(!obj.object_name().is_empty());
        }
    }

    #[test]
    fn iter_objects_on_empty_database() {
        let db = ObjectDatabase::new();
        assert_eq!(db.iter_objects().count(), 0);
    }

    #[test]
    fn duplicate_name_rejected() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object_typed(
            ObjectType::ANALOG_INPUT,
            1,
            "Sensor",
        ))
        .unwrap();
        // Different OID, same name → must fail
        let result = db.add(make_test_object_typed(
            ObjectType::ANALOG_INPUT,
            2,
            "Sensor",
        ));
        assert!(result.is_err());
        assert_eq!(db.len(), 1); // original still there
    }

    #[test]
    fn replace_same_oid_allowed() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object_typed(
            ObjectType::ANALOG_INPUT,
            1,
            "Sensor",
        ))
        .unwrap();
        // Same OID, same or different name → allowed (replacement)
        db.add(make_test_object_typed(
            ObjectType::ANALOG_INPUT,
            1,
            "Sensor-v2",
        ))
        .unwrap();
        assert_eq!(db.len(), 1);
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        assert_eq!(db.get(&oid).unwrap().object_name(), "Sensor-v2");
    }

    #[test]
    fn find_by_name_works() {
        let mut db = ObjectDatabase::new();
        db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "Temp"))
            .unwrap();
        db.add(make_test_object_typed(ObjectType::BINARY_INPUT, 1, "Alarm"))
            .unwrap();

        let obj = db.find_by_name("Temp").unwrap();
        assert_eq!(obj.object_identifier().instance_number(), 1);
        assert_eq!(
            obj.object_identifier().object_type(),
            ObjectType::ANALOG_INPUT
        );

        assert!(db.find_by_name("NonExistent").is_none());
    }

    #[test]
    fn remove_frees_name() {
        let mut db = ObjectDatabase::new();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        db.add(make_test_object_typed(
            ObjectType::ANALOG_INPUT,
            1,
            "Sensor",
        ))
        .unwrap();
        db.remove(&oid);
        // Name should now be available for a different object
        db.add(make_test_object_typed(
            ObjectType::ANALOG_INPUT,
            2,
            "Sensor",
        ))
        .unwrap();
        assert_eq!(db.len(), 1);
    }
}
