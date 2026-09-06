use std::borrow::Cow;

use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_types::enums::NotifyType;

use super::*;

struct AlarmSummaryFixture {
    oid: ObjectIdentifier,
    name: String,
    advertised: Vec<PropertyIdentifier>,
    values: Vec<(PropertyIdentifier, PropertyValue)>,
}

impl AlarmSummaryFixture {
    fn alarm(instance: u32) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            name: format!("ALARM-SUMMARY-{instance}"),
            advertised: vec![
                PropertyIdentifier::EVENT_STATE,
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyIdentifier::ACKED_TRANSITIONS,
            ],
            values: vec![
                (
                    PropertyIdentifier::EVENT_STATE,
                    PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
                ),
                (
                    PropertyIdentifier::NOTIFY_TYPE,
                    PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
                ),
                (
                    PropertyIdentifier::ACKED_TRANSITIONS,
                    transition_bits(0b111),
                ),
            ],
        }
    }

    fn advertise(&mut self, property: PropertyIdentifier) {
        if !self.advertised.contains(&property) {
            self.advertised.push(property);
        }
    }

    fn set(&mut self, property: PropertyIdentifier, value: PropertyValue) {
        self.values.retain(|(candidate, _)| *candidate != property);
        self.values.push((property, value));
    }

    fn remove(&mut self, property: PropertyIdentifier) {
        self.values.retain(|(candidate, _)| *candidate != property);
    }
}

impl BACnetObject for AlarmSummaryFixture {
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
            .find(|(candidate, _)| *candidate == property)
            .map(|(_, value)| value.clone())
            .ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
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
        Cow::Owned(self.advertised.clone())
    }
}

fn transition_bits(bits: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bacnet_types::bitstring::pack_octet(bits)],
    }
}

fn response(db: &ObjectDatabase) -> Result<GetAlarmSummaryAck, Error> {
    let mut encoded = BytesMut::new();
    handle_get_alarm_summary(db, &mut encoded)?;
    GetAlarmSummaryAck::decode(&encoded)
}

fn assert_operational_problem(error: Error) {
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::DEVICE.to_raw() as u32
                && code == ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32
    ));
}

fn add(db: &mut ObjectDatabase, object: AlarmSummaryFixture) {
    db.add(Box::new(object)).unwrap();
}

#[test]
fn selects_only_active_alarm_notify_type_and_preserves_output_fields() {
    let mut db = ObjectDatabase::new();
    let mut selected = AlarmSummaryFixture::alarm(1);
    selected.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(EventState::FAULT.to_raw()),
    );
    selected.set(
        PropertyIdentifier::ACKED_TRANSITIONS,
        transition_bits(0b010),
    );
    add(&mut db, selected);

    let mut informational = AlarmSummaryFixture::alarm(2);
    informational.set(
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
    );
    add(&mut db, informational);

    let mut normal_alarm = AlarmSummaryFixture::alarm(3);
    normal_alarm.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
    );
    add(&mut db, normal_alarm);

    let ack = response(&db).unwrap();
    assert_eq!(ack.entries.len(), 1);
    assert_eq!(ack.entries[0].object_identifier.instance_number(), 1);
    assert_eq!(ack.entries[0].alarm_state, EventState::FAULT);
    assert_eq!(
        ack.entries[0].acknowledged_transitions,
        (5, vec![0b0100_0000])
    );
}

#[test]
fn detection_false_is_excluded_absence_is_eligible_and_missing_signature_is_skipped() {
    let mut db = ObjectDatabase::new();
    let mut disabled = AlarmSummaryFixture::alarm(1);
    disabled.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    disabled.set(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    disabled.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Boolean(false),
    );
    disabled.set(
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyValue::Boolean(false),
    );
    disabled.set(
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyValue::Boolean(false),
    );
    add(&mut db, disabled);

    add(&mut db, AlarmSummaryFixture::alarm(2));

    let mut not_event_initiating = AlarmSummaryFixture::alarm(3);
    not_event_initiating
        .advertised
        .retain(|property| *property != PropertyIdentifier::NOTIFY_TYPE);
    not_event_initiating.values.clear();
    add(&mut db, not_event_initiating);

    let ack = response(&db).unwrap();
    assert_eq!(ack.entries.len(), 1);
    assert_eq!(ack.entries[0].object_identifier.instance_number(), 2);
}

#[test]
fn excluded_state_and_notify_types_short_circuit_malformed_later_fields() {
    let mut db = ObjectDatabase::new();
    let mut normal = AlarmSummaryFixture::alarm(1);
    normal.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
    );
    normal.set(
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyValue::Boolean(false),
    );
    normal.set(
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyValue::Boolean(false),
    );
    add(&mut db, normal);

    for (instance, notify_type) in [
        (2, NotifyType::EVENT.to_raw()),
        (3, NotifyType::ACK_NOTIFICATION.to_raw()),
        (4, 99),
    ] {
        let mut non_alarm = AlarmSummaryFixture::alarm(instance);
        non_alarm.set(
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(notify_type),
        );
        non_alarm.set(
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyValue::Boolean(false),
        );
        add(&mut db, non_alarm);
    }

    let mut encoded = BytesMut::new();
    handle_get_alarm_summary(&db, &mut encoded).unwrap();
    assert!(encoded.is_empty());
    assert!(GetAlarmSummaryAck::decode(&encoded)
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn unreadable_and_wrongly_typed_advertised_fields_are_operational_problems() {
    for (property, value) in [
        (
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            Some(PropertyValue::Unsigned(1)),
        ),
        (PropertyIdentifier::EVENT_DETECTION_ENABLE, None),
        (
            PropertyIdentifier::EVENT_STATE,
            Some(PropertyValue::Boolean(true)),
        ),
        (PropertyIdentifier::EVENT_STATE, None),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            Some(PropertyValue::Boolean(true)),
        ),
        (PropertyIdentifier::NOTIFY_TYPE, None),
        (
            PropertyIdentifier::ACKED_TRANSITIONS,
            Some(PropertyValue::Boolean(true)),
        ),
        (PropertyIdentifier::ACKED_TRANSITIONS, None),
    ] {
        let mut object = AlarmSummaryFixture::alarm(1);
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            object.advertise(property);
        }
        object.remove(property);
        if let Some(value) = value {
            object.set(property, value);
        }
        let mut db = ObjectDatabase::new();
        add(&mut db, object);
        assert_operational_problem(response(&db).unwrap_err());
    }
}

#[test]
fn acknowledged_transitions_requires_one_canonical_three_bit_octet() {
    for malformed in [
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b1110_0000],
        },
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000, 0],
        },
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0001],
        },
    ] {
        let mut object = AlarmSummaryFixture::alarm(1);
        object.set(PropertyIdentifier::ACKED_TRANSITIONS, malformed);
        let mut db = ObjectDatabase::new();
        add(&mut db, object);
        assert_operational_problem(response(&db).unwrap_err());
    }
}
