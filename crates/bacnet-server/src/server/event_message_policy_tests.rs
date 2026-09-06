use super::*;
use crate::server::event_notifications::CommittedIntrinsicTransition;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::event::TransitionOutcome;
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, EventType};

fn builtin_intrinsic_objects() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(AnalogInputObject::new(41, "localized analog input", 0).unwrap()),
        Box::new(AnalogOutputObject::new(41, "localized analog output", 0).unwrap()),
        Box::new(AnalogValueObject::new(41, "localized analog value", 0).unwrap()),
        Box::new(BinaryInputObject::new(41, "localized binary input").unwrap()),
        Box::new(BinaryOutputObject::new(41, "localized binary output").unwrap()),
        Box::new(BinaryValueObject::new(41, "localized binary value").unwrap()),
        Box::new(MultiStateInputObject::new(41, "localized multistate input", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(41, "localized multistate output", 3).unwrap()),
        Box::new(MultiStateValueObject::new(41, "localized multistate value", 3).unwrap()),
    ]
}

fn commit(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    from: EventState,
    to: EventState,
    distribute: bool,
) -> Option<CommittedIntrinsicTransition> {
    BACnetServer::<RecordingTransport>::commit_intrinsic_transition(
        db,
        &oid,
        TransitionOutcome {
            change: EventStateChange { from, to },
            event_type: EventType::OUT_OF_RANGE,
            distribute,
        },
    )
}

fn message_slots(db: &ObjectDatabase, oid: ObjectIdentifier) -> [String; 3] {
    std::array::from_fn(|index| {
        let PropertyValue::CharacterString(text) = db
            .get(&oid)
            .unwrap()
            .read_property(
                PropertyIdentifier::EVENT_MESSAGE_TEXTS,
                Some(index as u32 + 1),
            )
            .unwrap()
        else {
            panic!("Event_Message_Texts coordinate must be a character string");
        };
        text
    })
}

fn committed_properties(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<PropertyValue> {
    let object = db.get(&oid).unwrap();
    let mut values = vec![
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
    ];
    for property in [
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
    ] {
        values.extend((1..=3).map(|index| object.read_property(property, Some(index)).unwrap()));
    }
    values
}

#[test]
fn all_nine_builtin_families_store_each_policy_message_in_only_its_coordinate() {
    let objects = builtin_intrinsic_objects();
    assert_eq!(objects.len(), 9);

    for object in objects {
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let cases = [
            (EventState::NORMAL, EventState::OFFNORMAL),
            (EventState::OFFNORMAL, EventState::FAULT),
            (EventState::FAULT, EventState::NORMAL),
        ];
        let mut expected = std::array::from_fn(|_| String::new());

        for (index, (from, to)) in cases.into_iter().enumerate() {
            assert!(
                commit(&mut db, oid, from, to, false).is_some(),
                "{oid} must use the built-in atomic commit path"
            );
            expected[index] = format!("{oid}: {from} -> {to}");
            assert_eq!(
                message_slots(&db, oid),
                expected,
                "{oid} must update only transition coordinate {index}"
            );
        }
    }
}

#[test]
fn policy_format_uses_object_and_state_display_including_unknown_state_numbers() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    let unknown = EventState::from_raw(65_535);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(7, "name is not policy", 0).unwrap(),
    ))
    .unwrap();

    let committed = commit(&mut db, oid, EventState::NORMAL, unknown, true)
        .expect("the local transition and message commit independently of wire projection");
    assert!(
        !crate::server::event_notifications::ResolvedIntrinsicTransition::Committed(committed)
            .can_emit(),
        "an unsupported structured projection suppresses only the outbound frame"
    );
    assert_eq!(
        message_slots(&db, oid),
        [
            "ANALOG_INPUT,7: NORMAL -> 65535".into(),
            String::new(),
            String::new(),
        ]
    );
}

#[test]
fn stale_commit_does_not_mutate_committed_event_properties() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 8).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AnalogInputObject::new(8, "AI-8", 0).unwrap()))
        .unwrap();
    assert!(commit(
        &mut db,
        oid,
        EventState::NORMAL,
        EventState::HIGH_LIMIT,
        true,
    )
    .is_some());
    let before = committed_properties(&db, oid);

    assert!(commit(&mut db, oid, EventState::NORMAL, EventState::FAULT, true,).is_none());
    assert_eq!(committed_properties(&db, oid), before);
}

#[tokio::test]
async fn outbound_message_text_equals_the_committed_history_coordinate() {
    let db = db_with_high_limit_transition(0x80);
    let sent = broadcasts_from_per_write_path(&db, 0).await;
    let notification = decode_broadcast_notification(&StdMutex::new(sent));
    let expected = "ANALOG_INPUT,1: NORMAL -> HIGH_LIMIT";
    let history = message_slots(
        &*db.read().await,
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
    );

    assert_eq!(history, [expected.into(), String::new(), String::new()]);
    assert_eq!(notification.message_text, Some(expected.into()));
}

#[tokio::test]
async fn event_enable_and_dcc_suppression_still_commit_the_policy_message() {
    for (event_enable, dcc, label) in [
        (0x00, 0, "Event_Enable"),
        (0x80, 1, "device communication control"),
    ] {
        let db = db_with_high_limit_transition(event_enable);
        let sent = broadcasts_from_per_write_path(&db, dcc).await;
        assert!(sent.is_empty(), "{label} must suppress distribution");
        assert_eq!(
            message_slots(
                &*db.read().await,
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            ),
            [
                "ANALOG_INPUT,1: NORMAL -> HIGH_LIMIT".into(),
                String::new(),
                String::new(),
            ],
            "{label} must not suppress the local message-history commit"
        );
    }
}
