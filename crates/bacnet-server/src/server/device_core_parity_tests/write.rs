//! WriteProperty through the server and the `Responder`: the same reply
//! octets for each row, and the same value read back after it, except the
//! two exchanges [`DIFFER_EXCHANGES`] pins.
use super::*;
use bacnet_objects::command::CommandObject;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use std::borrow::Cow;

fn bv(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, instance)
}

fn cmd1() -> ObjectIdentifier {
    oid(ObjectType::COMMAND, 1)
}

/// A custom Device-typed object, instance 4000000, whose writer takes every
/// value, so only the executor-owned gate refuses a write to it.
struct Permissive;

impl bacnet_objects::traits::BACnetObject for Permissive {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(ObjectType::DEVICE, 4_000_000)
    }
    fn object_name(&self) -> &str {
        "permissive"
    }
    fn read_property(&self, _: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Null)
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        crate::test_rows::rows(&[P::DESCRIPTION], &[P::DESCRIPTION])
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        bacnet_objects::property_metadata::property_list_from_metadata(&self.property_metadata())
    }
}

/// The read fixture, with BV-2 (a writable, not commandable, Present_Value),
/// CMD-1, whose list 1 writes BV-2 active at priority 8, and [`Permissive`].
fn writes(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let writable = BinaryValueObject::with_access(2, "BV-2", PresentValueAccess::Writable).unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![BACnetActionCommand {
                device_identifier: None,
                object_identifier: bv(2),
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Enumerated(1),
                priority: Some(8),
                post_delay: None,
                quit_on_failure: false,
                write_successful: true,
            }],
        }])
        .unwrap();
    database_with(
        identity,
        vec![Box::new(writable), Box::new(command), Box::new(Permissive)],
    )
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut octets = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut octets, value).unwrap();
    octets.to_vec()
}

fn write_property(
    invoke_id: u8,
    object: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: &PropertyValue,
) -> Bytes {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: index,
        property_value: encoded(value),
        priority: Some(8),
    }
    .encode(&mut body)
    .unwrap();
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &body,
        1476,
    )
}

fn is_simple_ack(reply: &Option<Apdu>) -> bool {
    matches!(reply, Some(Apdu::SimpleAck(ack))
        if ack.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY)
}

impl Parity {
    async fn start_writes() -> Self {
        Self::start_from(
            ReadPropertyMultipleBudget::default(),
            Segmentation::NONE,
            writes,
        )
        .await
    }

    /// Both sides' `property` of `object`, required to agree.
    async fn read_back(&mut self, what: &str, object: ObjectIdentifier, property: P) -> Apdu {
        let apdu = read_property(1, object, property, None);
        let reply = self.same(what, apdu, 1476).await;
        assert!(
            matches!(reply, Some(Apdu::ComplexAck(_))),
            "{what}: {reply:?}"
        );
        reply.unwrap()
    }
}

#[tokio::test(start_paused = true)]
async fn write_property_rows_match() {
    let mut parity = Parity::start_writes().await;
    let active = PropertyValue::Enumerated(1);

    let write = write_property(1, bv(2), P::PRESENT_VALUE, None, &active);
    let reply = parity.same("a successful write", write, 1476).await;
    assert!(is_simple_ack(&reply), "{reply:?}");
    parity
        .read_back("the written value", bv(2), P::PRESENT_VALUE)
        .await;

    let device = oid(ObjectType::DEVICE, DEVICE);
    let vendor = write_property(
        2,
        device,
        P::VENDOR_IDENTIFIER,
        None,
        &PropertyValue::Unsigned(7),
    );
    let reply = parity.same("WRITE_ACCESS_DENIED", vendor, 1476).await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );

    // A Device writer that would take it never sees it.
    let permissive = oid(ObjectType::DEVICE, 4_000_000);
    for property in [P::PROTOCOL_SERVICES_SUPPORTED, P::VENDOR_IDENTIFIER] {
        let owned = write_property(2, permissive, property, None, &PropertyValue::Unsigned(7));
        let reply = parity
            .same("WRITE_ACCESS_DENIED, custom Device", owned, 1476)
            .await;
        assert_eq!(
            error_of(reply),
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
            "{property:?}"
        );
    }

    let null = write_property(3, bv(1), P::OBJECT_NAME, None, &PropertyValue::Null);
    let reply = parity.same("a NULL left unchanged", null, 1476).await;
    assert!(is_simple_ack(&reply), "{reply:?}");
    parity
        .read_back("the name a NULL left", bv(1), P::OBJECT_NAME)
        .await;

    let taken = PropertyValue::CharacterString("AI-1".into());
    let duplicate = write_property(4, bv(2), P::OBJECT_NAME, None, &taken);
    let reply = parity
        .same("a duplicate Object_Name", duplicate, 1476)
        .await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::PROPERTY, ErrorCode::DUPLICATE_NAME)
    );

    let indexed = write_property(5, bv(2), P::PRESENT_VALUE, Some(1), &active);
    let reply = parity.same("an index on a non-array", indexed, 1476).await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
    );

    let missing = write_property(6, bv(9), P::PRESENT_VALUE, None, &active);
    let reply = parity.same("an unknown object", missing, 1476).await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
    );

    // The object and property, then no value.
    let malformed = confirmed(
        7,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &[0x0C, 0x01, 0x40, 0x00, 0x02, 0x19, 0x55],
        1476,
    );
    let reply = parity
        .same("malformed write parameters", malformed, 1476)
        .await;
    assert!(matches!(reply, Some(Apdu::Reject(_))), "{reply:?}");

    let body = write_property(8, bv(2), P::PRESENT_VALUE, None, &active);
    let segmented = confirmed_with(
        8,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &body[4..],
        1476,
        true,
        false,
    );
    let reply = parity.same("a segmented write", segmented, 1476).await;
    assert!(matches!(reply, Some(Apdu::Abort(_))), "{reply:?}");
    parity.server.stop().await.unwrap();
}

/// A Command's Present_Value write: both acknowledge it, and both leave
/// the Command out of process. The server performs the run's action, the
/// core ends the run unmade and performs none, so the action's effect
/// differs: BV-2's Present_Value, the action's write-successful flag, and
/// the Command's All_Writes_Successful.
#[tokio::test(start_paused = true)]
async fn a_command_write_matches_until_its_run_ends() {
    let mut parity = Parity::start_writes().await;
    let select = write_property(
        1,
        cmd1(),
        P::PRESENT_VALUE,
        None,
        &PropertyValue::Unsigned(1),
    );
    let reply = parity
        .same("a Command Present_Value write", select, 1476)
        .await;
    assert!(is_simple_ack(&reply), "{reply:?}");
    parity
        .read_back("In_Process after the run", cmd1(), P::IN_PROCESS)
        .await;
    const ACTIONS: &str = "a Command run's actions";
    let value = |reply: Option<Apdu>| {
        let Some(Apdu::ComplexAck(ack)) = reply else {
            panic!("expected a ComplexACK, got {reply:?}");
        };
        let ack =
            bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack).unwrap();
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0)
            .unwrap()
            .0
    };
    // The action's target: written active by the server, untouched by the
    // core.
    let target = read_property(2, bv(2), P::PRESENT_VALUE, None);
    let (server, core) = parity.differ(ACTIONS, target, 1476).await;
    assert_eq!(
        (value(server), value(core)),
        (PropertyValue::Enumerated(1), PropertyValue::Enumerated(0)),
        "BV-2's Present_Value: server, then core"
    );
    // List 1, whose one action records whether its write succeeded.
    let action = read_property(3, cmd1(), P::ACTION, Some(1));
    let (server, core) = parity.differ(ACTIONS, action, 1476).await;
    let succeeded = |reply: Option<Apdu>| {
        let Some(Apdu::ComplexAck(ack)) = reply else {
            panic!("expected a ComplexACK, got {reply:?}");
        };
        let ack =
            bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack).unwrap();
        let (list, _) =
            bacnet_encoding::constructed::decode_action_list(&ack.property_value, 0).unwrap();
        list.commands
            .iter()
            .map(|command| command.write_successful)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        (succeeded(server), succeeded(core)),
        (vec![true], vec![false]),
        "the action's write-successful flag: server, then core"
    );
    let flag = read_property(4, cmd1(), P::ALL_WRITES_SUCCESSFUL, None);
    let (server, core) = parity.differ(ACTIONS, flag, 1476).await;
    assert_eq!(
        (value(server), value(core)),
        (PropertyValue::Boolean(true), PropertyValue::Boolean(false)),
        "All_Writes_Successful: server, then core"
    );
    parity.server.stop().await.unwrap();
}

/// A commandable Present_Value: the server commands it for the requester
/// its source address names; the core has none to name, and the object
/// refuses an unsourced command.
#[tokio::test(start_paused = true)]
async fn a_commandable_present_value_write_differs() {
    let mut parity = Parity::start_writes().await;
    let active = PropertyValue::Enumerated(1);
    let command = write_property(1, bv(1), P::PRESENT_VALUE, None, &active);
    let (server, core) = parity
        .differ("commandable Present_Value write", command, 1476)
        .await;
    assert!(is_simple_ack(&server), "{server:?}");
    assert_eq!(
        error_of(core),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    parity.server.stop().await.unwrap();
}
