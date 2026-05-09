use super::*;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;

// ─────── Mock object for testing ───────

/// A minimal mock object that stores properties in a HashMap for flexible
/// testing of all five algorithms without depending on real object types.
struct MockObject {
    oid: ObjectIdentifier,
    name: String,
    props: HashMap<PropertyIdentifier, PropertyValue>,
}

impl MockObject {
    fn new(oid: ObjectIdentifier) -> Self {
        Self {
            name: format!("mock-{}", oid),
            oid,
            props: HashMap::new(),
        }
    }

    fn set(&mut self, prop: PropertyIdentifier, val: PropertyValue) {
        self.props.insert(prop, val);
    }
}

impl BACnetObject for MockObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Owned(self.props.keys().copied().collect())
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.props.get(&property).cloned().ok_or(Error::Protocol {
            class: 2, // PROPERTY
            code: 32, // UNKNOWN_PROPERTY
        })
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        self.props.insert(property, value);
        Ok(())
    }
}

// ─────── Helpers ───────

fn make_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance).unwrap()
}

fn make_analog_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn setup_change_of_state(pv: u32, alarm_values: Vec<u32>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = make_oid(1);
    let mut obj = MockObject::new(oid);
    obj.set(
        PropertyIdentifier::EVENT_TYPE,
        PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
    );
    obj.set(
        PropertyIdentifier::EVENT_ENABLE,
        PropertyValue::Unsigned(0x07),
    );
    obj.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
    );
    obj.set(
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Enumerated(pv),
    );
    obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));
    let alarm_pvs: Vec<PropertyValue> = alarm_values
        .into_iter()
        .map(PropertyValue::Enumerated)
        .collect();
    obj.set(
        PropertyIdentifier::ALARM_VALUES,
        PropertyValue::List(alarm_pvs),
    );

    let mut db = ObjectDatabase::new();
    db.add(Box::new(obj)).unwrap();
    (db, oid)
}

// ═══════════════════════════════════════════════════════════════════════
// T38: CHANGE_OF_STATE
// ═══════════════════════════════════════════════════════════════════════

mod bitstring;
mod change_of_state;
mod change_of_value;
mod command_failure;
mod floating_limit;
