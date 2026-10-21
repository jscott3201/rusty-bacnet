use super::*;
use bacnet_types::constructed::BACnetActionCommand;
use bacnet_types::enums::{ErrorClass, ErrorCode};

/// The commands a queued run makes.
fn commands(run: &CommandRun) -> &[BACnetActionCommand] {
    match &run.plan {
        RunPlan::Actions(commands) => commands,
        other => panic!("a Command queues its list, not {other:?}"),
    }
}

fn assert_property_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected {expected:?}, got {result:?}"
    );
}

#[test]
fn command_create_and_read_defaults() {
    let cmd = CommandObject::new(1, "CMD-1").unwrap();
    assert_eq!(cmd.object_name(), "CMD-1");
    assert_eq!(
        cmd.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(
        cmd.read_property(PropertyIdentifier::IN_PROCESS, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        cmd.read_property(PropertyIdentifier::ALL_WRITES_SUCCESSFUL, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
}

#[test]
fn command_object_type() {
    let cmd = CommandObject::new(1, "CMD-1").unwrap();
    assert_eq!(
        cmd.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::COMMAND.to_raw())
    );
}

#[test]
fn command_write_present_value() {
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    cmd.set_action(vec![BACnetActionList::default(); 3])
        .unwrap();
    cmd.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    assert_eq!(
        cmd.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
}

#[test]
fn command_write_present_value_wrong_type() {
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    let result = cmd.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn command_action_read_only() {
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    cmd.set_action(vec![BACnetActionList::default()]).unwrap();
    // Whole or one element, Action has no write route.
    for index in [None, Some(0), Some(1)] {
        let value = PropertyValue::ApplicationData(vec![0x0E, 0x0F]);
        assert_property_error(
            cmd.write_property(PropertyIdentifier::ACTION, index, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(
        cmd.read_property(PropertyIdentifier::ACTION, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn command_read_action_empty() {
    let cmd = CommandObject::new(1, "CMD-1").unwrap();
    assert_eq!(
        cmd.read_property(PropertyIdentifier::ACTION, None).unwrap(),
        PropertyValue::List(vec![])
    );
}

fn write_ao1(priority: Option<u8>) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(50.0),
        priority,
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    }
}

#[test]
fn command_read_action_serves_one_framed_action_list_per_index() {
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    let lists = vec![
        BACnetActionList {
            commands: vec![write_ao1(Some(8))],
        },
        BACnetActionList::default(),
    ];
    cmd.set_action(lists.clone()).unwrap();
    let elements: Vec<PropertyValue> = lists
        .iter()
        .map(|list| {
            let mut encoded = BytesMut::new();
            encode_action_list(&mut encoded, list).unwrap();
            PropertyValue::ApplicationData(encoded.to_vec())
        })
        .collect();
    assert_eq!(
        cmd.read_property(PropertyIdentifier::ACTION, None).unwrap(),
        PropertyValue::List(elements.clone())
    );
    assert_eq!(
        cmd.read_property(PropertyIdentifier::ACTION, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    for (index, element) in (1..).zip(&elements) {
        assert_eq!(
            &cmd.read_property(PropertyIdentifier::ACTION, Some(index))
                .unwrap(),
            element
        );
    }
    // The empty list is its [0] frame alone.
    assert_eq!(
        elements[1],
        PropertyValue::ApplicationData(vec![0x0E, 0x0F])
    );
    for index in [3, u32::MAX] {
        assert_property_error(
            cmd.read_property(PropertyIdentifier::ACTION, Some(index)),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn command_set_action_refuses_a_priority_outside_one_to_sixteen() {
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    let kept = vec![BACnetActionList {
        commands: vec![write_ao1(Some(16))],
    }];
    cmd.set_action(kept).unwrap();
    let before = cmd.read_property(PropertyIdentifier::ACTION, None).unwrap();
    for priority in [0, 17] {
        let refused = vec![BACnetActionList {
            commands: vec![write_ao1(Some(1)), write_ao1(Some(priority))],
        }];
        assert_property_error(cmd.set_action(refused), ErrorCode::VALUE_OUT_OF_RANGE);
        assert_eq!(
            cmd.read_property(PropertyIdentifier::ACTION, None).unwrap(),
            before
        );
    }
}

#[test]
fn command_property_list() {
    let cmd = CommandObject::new(1, "CMD-1").unwrap();
    let list = cmd.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::IN_PROCESS));
    assert!(list.contains(&PropertyIdentifier::ALL_WRITES_SUCCESSFUL));
    assert!(list.contains(&PropertyIdentifier::ACTION));
}

fn write_pv(cmd: &mut CommandObject, value: u64) -> Result<(), Error> {
    cmd.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(value),
        None,
    )
}

fn read_bool(cmd: &CommandObject, property: PropertyIdentifier) -> bool {
    match cmd.read_property(property, None).unwrap() {
        PropertyValue::Boolean(value) => value,
        other => panic!("{property:?} read {other:?}"),
    }
}

/// In_Process and All_Writes_Successful, in that order.
fn state(cmd: &CommandObject) -> (bool, bool) {
    (
        read_bool(cmd, PropertyIdentifier::IN_PROCESS),
        read_bool(cmd, PropertyIdentifier::ALL_WRITES_SUCCESSFUL),
    )
}

/// The write-successful flags of Action element `index` (one-based).
fn flags(cmd: &CommandObject, index: u32) -> Vec<bool> {
    let PropertyValue::ApplicationData(bytes) = cmd
        .read_property(PropertyIdentifier::ACTION, Some(index))
        .unwrap()
    else {
        panic!("Action element {index} is framed bytes");
    };
    let (list, _) = bacnet_encoding::constructed::decode_action_list(&bytes, 0).unwrap();
    list.commands
        .iter()
        .map(|command| command.write_successful)
        .collect()
}

fn quitting(mut command: BACnetActionCommand) -> BACnetActionCommand {
    command.quit_on_failure = true;
    command
}

/// List 1 holds three writes, the first quitting on failure; list 2 is empty.
fn configured() -> CommandObject {
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    cmd.set_action(vec![
        BACnetActionList {
            commands: vec![
                quitting(write_ao1(Some(8))),
                write_ao1(None),
                write_ao1(None),
            ],
        },
        BACnetActionList::default(),
    ])
    .unwrap();
    cmd
}

#[test]
fn command_present_value_write_queues_the_selected_list_and_busies_the_object() {
    let mut cmd = configured();
    assert!(cmd.take_command_run_internal().is_none());
    write_pv(&mut cmd, 1).unwrap();
    assert_eq!(state(&cmd), (true, false));
    let run = cmd.take_command_run_internal().unwrap();
    assert_eq!(run.source, cmd.object_identifier());
    assert_eq!(Some(run.generation), cmd.command_generation_internal());
    assert_eq!(commands(&run).len(), 3);
    assert!(commands(&run)[0].quit_on_failure);
    assert!(cmd.take_command_run_internal().is_none(), "taken once");

    // Any Present_Value write while the list runs, the same number included,
    // is OBJECT / BUSY and changes nothing.
    for value in [0, 1, 2] {
        let result = write_pv(&mut cmd, value);
        assert!(
            matches!(result, Err(Error::Protocol { class, code })
                if class == ErrorClass::OBJECT
                    && code == ErrorCode::BUSY),
            "{value}: {result:?}"
        );
    }
    assert_eq!(Some(run.generation), cmd.command_generation_internal());

    for command in 0..3 {
        assert!(cmd.record_command_write_internal(run.generation, command, true));
    }
    assert_eq!(state(&cmd), (true, false), "still running until completed");
    assert!(cmd.complete_command_run_internal(run.generation, Ok(())));
    assert_eq!(state(&cmd), (false, true));
    assert_eq!(flags(&cmd, 1), [true, true, true]);
    assert!(!cmd.complete_command_run_internal(run.generation, Ok(())));

    // Writing the same number again starts the list again.
    write_pv(&mut cmd, 1).unwrap();
    let again = cmd.take_command_run_internal().unwrap();
    assert_ne!(again.generation, run.generation);
    assert_eq!(commands(&again), commands(&run));
}

#[test]
fn command_quit_on_failure_marks_the_commands_after_it_unsuccessful() {
    let mut cmd = configured();
    write_pv(&mut cmd, 1).unwrap();
    let run = cmd.take_command_run_internal().unwrap();
    // The first command fails and quits: the other two are never made.
    assert!(cmd.record_command_write_internal(run.generation, 0, false));
    assert_eq!(flags(&cmd, 1), [false, false, false]);
    assert!(cmd.complete_command_run_internal(run.generation, Err(WriteFailure::Process)));
    assert_eq!(state(&cmd), (false, false));

    // A failure that doesn't quit marks that command alone.
    write_pv(&mut cmd, 1).unwrap();
    let run = cmd.take_command_run_internal().unwrap();
    assert!(cmd.record_command_write_internal(run.generation, 0, true));
    assert!(cmd.record_command_write_internal(run.generation, 1, false));
    assert!(cmd.record_command_write_internal(run.generation, 2, true));
    assert_eq!(flags(&cmd, 1), [true, false, true]);
    assert!(!cmd.record_command_write_internal(run.generation, 3, true));
    assert!(cmd.complete_command_run_internal(run.generation, Err(WriteFailure::Process)));
    assert_eq!(state(&cmd), (false, false));
}

#[test]
fn command_zero_and_an_empty_list_complete_at_once() {
    let mut cmd = configured();
    write_pv(&mut cmd, 1).unwrap();
    let run = cmd.take_command_run_internal().unwrap();
    assert!(cmd.complete_command_run_internal(run.generation, Err(WriteFailure::Process)));
    assert_eq!(state(&cmd), (false, false));
    for value in [0, 2] {
        write_pv(&mut cmd, value).unwrap();
        assert!(cmd.take_command_run_internal().is_none(), "{value}");
        assert_eq!(state(&cmd), (false, true), "{value}");
        assert_eq!(
            cmd.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(value)
        );
    }
}

#[test]
fn command_present_value_above_the_action_size_is_out_of_range() {
    let mut cmd = configured();
    write_pv(&mut cmd, 2).unwrap();
    for value in [3, u64::MAX] {
        assert_property_error(write_pv(&mut cmd, value), ErrorCode::VALUE_OUT_OF_RANGE);
    }
    assert_eq!(
        cmd.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert!(cmd.take_command_run_internal().is_none());
    assert_property_error(
        write_pv(&mut CommandObject::new(2, "CMD-2").unwrap(), 1),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}

#[test]
fn command_set_action_abandons_a_run_and_its_reports_go_stale() {
    let mut cmd = configured();
    write_pv(&mut cmd, 1).unwrap();
    let run = cmd.take_command_run_internal().unwrap();
    cmd.set_action(vec![BACnetActionList {
        commands: vec![write_ao1(None)],
    }])
    .unwrap();
    assert!(!state(&cmd).0);
    assert_ne!(Some(run.generation), cmd.command_generation_internal());
    assert!(!cmd.record_command_write_internal(run.generation, 0, false));
    assert!(!cmd.complete_command_run_internal(run.generation, Err(WriteFailure::Process)));
    assert_eq!(flags(&cmd, 1), [true]);
    // The new list starts as usual.
    write_pv(&mut cmd, 1).unwrap();
    assert!(cmd.take_command_run_internal().is_some());
}

#[test]
fn command_action_text_serves_one_text_per_list_and_follows_the_action_size() {
    let mut cmd = configured();
    assert_property_error(
        cmd.read_property(PropertyIdentifier::ACTION_TEXT, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_property_error(
        cmd.set_action_text(vec!["Occupied".into()]),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    cmd.set_action_text(vec!["Occupied".into(), "Unoccupied".into()])
        .unwrap();
    let text = |s: &str| PropertyValue::CharacterString(s.into());
    let read =
        |cmd: &CommandObject, index| cmd.read_property(PropertyIdentifier::ACTION_TEXT, index);
    assert_eq!(
        read(&cmd, None).unwrap(),
        PropertyValue::List(vec![text("Occupied"), text("Unoccupied")])
    );
    assert_eq!(read(&cmd, Some(0)).unwrap(), PropertyValue::Unsigned(2));
    assert_eq!(read(&cmd, Some(2)).unwrap(), text("Unoccupied"));
    assert_property_error(read(&cmd, Some(3)), ErrorCode::INVALID_ARRAY_INDEX);

    // Action grows: Action_Text grows with empty texts.
    cmd.set_action(vec![BACnetActionList::default(); 3])
        .unwrap();
    assert_eq!(
        read(&cmd, None).unwrap(),
        PropertyValue::List(vec![text("Occupied"), text("Unoccupied"), text("")])
    );
    // Action shrinks: Action_Text keeps its leading texts.
    cmd.set_action(vec![BACnetActionList::default()]).unwrap();
    assert_eq!(
        read(&cmd, None).unwrap(),
        PropertyValue::List(vec![text("Occupied")])
    );
}

#[test]
fn command_takes_property_subscriptions_but_not_subscribe_cov() {
    let cmd = CommandObject::new(1, "CMD-1").unwrap();
    assert!(!cmd.supports_cov());
    assert!(cmd.supports_subscribe_cov_property());
    assert!(cmd.supports_cov_property(PropertyIdentifier::IN_PROCESS));
}

#[test]
fn command_replacement_never_shares_a_run_generation() {
    // Two objects with one identifier, each written once: a run left over
    // from the first can't pass for the second's.
    let mut old = configured();
    write_pv(&mut old, 1).unwrap();
    let stale = old.take_command_run_internal().unwrap();
    let mut fresh = configured();
    write_pv(&mut fresh, 1).unwrap();
    let current = fresh.take_command_run_internal().unwrap();
    assert_eq!(stale.source, current.source);
    assert_ne!(stale.generation, current.generation);
    assert!(!fresh.record_command_write_internal(stale.generation, 0, false));
    assert!(!fresh.complete_command_run_internal(stale.generation, Err(WriteFailure::Process)));
    assert_eq!(state(&fresh), (true, false));
    assert!(fresh.complete_command_run_internal(current.generation, Ok(())));
}
