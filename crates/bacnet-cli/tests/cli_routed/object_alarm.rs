//! Object mutations and canonical alarm acknowledgment through the real router.
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::{EventTransitionBits, LimitEnable};
use bacnet_types::enums::{EngineeringUnits, EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

pub(super) fn database() -> ObjectDatabase {
    let mut db = read_services::database();
    let mut alarm =
        AnalogInputObject::new(77, "Routed alarm", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, PropertyValue::Real(80.0)),
        (PropertyIdentifier::LOW_LIMIT, PropertyValue::Real(0.0)),
        (PropertyIdentifier::DEADBAND, PropertyValue::Real(1.0)),
        (
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::all().to_bacnet()],
            },
        ),
    ] {
        alarm.write_property(property, None, value, None).unwrap();
    }
    // The running detector sees the same high-limit condition as the committed history.
    alarm.set_present_value(81.0);
    alarm
        .commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            coordinate: EventTransition::ToOffnormal,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            message_text: Some("high limit committed".into()),
        })
        .unwrap();
    db.add(Box::new(alarm)).unwrap();
    db
}

async fn invoke(
    port: Option<u16>,
    target: &str,
    service: &str,
    arguments: &[&str],
) -> std::process::Output {
    invoke_at(port, target, service, arguments, None).await
}

async fn invoke_at(
    port: Option<u16>,
    target: &str,
    service: &str,
    arguments: &[&str],
    source_port: Option<u16>,
) -> std::process::Output {
    let mut cmd = command();
    cmd.args([
        "--interface",
        "127.0.0.1",
        "--port",
        &source_port.unwrap_or(0).to_string(),
        "--json",
    ]);
    if let Some(port) = port {
        cmd.args(["--foreign-bbmd", &format!("127.0.0.1:{port}")]);
    }
    cmd.arg(service).arg(target).args(arguments);
    let process = Process::spawn(&mut cmd);
    if source_port.is_some() {
        support::paused::output(process).await
    } else {
        process.output().await
    }
}

fn success(output: &std::process::Output, message: &str) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"status":"ok", "message":message})
    );
}

fn object_values(db: &ObjectDatabase) -> Vec<PropertyValue> {
    let object = db.get(&oid(ObjectType::BINARY_INPUT, 100)).unwrap();
    [
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyIdentifier::PRESENT_VALUE,
    ]
    .map(|property| object.read_property(property, None).unwrap())
    .to_vec()
}

pub(super) async fn object_lifecycle<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    port: Option<u16>,
    target: &str,
    source_port: Option<u16>,
) {
    let created = oid(ObjectType::BINARY_INPUT, 100);
    let before = server.database().read().await.len();
    assert!(server.database().read().await.get(&created).is_none());
    let output = invoke_at(
        port,
        target,
        "create-object",
        &["bi:100", "--allow-write"],
        source_port,
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "status": "ok",
            "message": "Created BINARY_INPUT:100",
            "object_type": "BINARY_INPUT",
            "instance": 100,
        })
    );
    let expected = vec![
        PropertyValue::ObjectIdentifier(created),
        PropertyValue::Enumerated(0),
    ];
    assert_eq!(object_values(&*server.database().read().await), expected);
    assert_eq!(server.database().read().await.len(), before + 1);
    failure(
        &invoke_at(
            port,
            target,
            "create-object",
            &["bi:100", "--allow-write"],
            source_port,
        )
        .await,
        "OBJECT_IDENTIFIER_ALREADY_EXISTS",
    );
    assert_eq!(object_values(&*server.database().read().await), expected);
    assert_eq!(server.database().read().await.len(), before + 1);
    failure(
        &invoke_at(
            port,
            target,
            "delete-object",
            &[&format!("device:{INSTANCE}"), "--allow-write"],
            source_port,
        )
        .await,
        "OBJECT_DELETION_NOT_PERMITTED",
    );
    assert_eq!(object_values(&*server.database().read().await), expected);
    assert_eq!(server.database().read().await.len(), before + 1);
    success(
        &invoke_at(
            port,
            target,
            "delete-object",
            &["bi:100", "--allow-write"],
            source_port,
        )
        .await,
        "Deleted BINARY_INPUT:100",
    );
    assert!(server.database().read().await.get(&created).is_none());
    failure(
        &invoke_at(
            port,
            target,
            "delete-object",
            &["bi:100", "--allow-write"],
            source_port,
        )
        .await,
        "UNKNOWN_OBJECT",
    );
    assert_eq!(server.database().read().await.len(), before);
    assert!(
        server
            .database()
            .read()
            .await
            .get(&oid(ObjectType::DEVICE, INSTANCE))
            .is_some()
    );
}

fn alarm_snapshot(db: &ObjectDatabase) -> (Vec<PropertyValue>, EventTransitionBits) {
    let object = db.get(&oid(ObjectType::ANALOG_INPUT, 77)).unwrap();
    let history = [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
    ]
    .map(|property| object.read_property(property, None).unwrap())
    .to_vec();
    let PropertyValue::BitString { unused_bits, data } = object
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    else {
        panic!("expected Acked_Transitions bitstring");
    };
    assert_eq!(unused_bits, 5);
    (history, EventTransitionBits::from_bacnet(&data))
}

async fn acknowledge<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    port: Option<u16>,
    target: &str,
) {
    let before = alarm_snapshot(&*server.database().read().await);
    assert_eq!(
        before.0[0],
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
    assert_eq!(
        before.1,
        EventTransitionBits::TO_FAULT | EventTransitionBits::TO_NORMAL
    );
    for (state, timestamp, error) in [
        ("4", "sequence:42", "INVALID_EVENT_STATE"),
        ("3", "sequence:41", "INVALID_TIME_STAMP"),
    ] {
        failure(
            &invoke(
                port,
                target,
                "ack-alarm",
                &[
                    "ai:77",
                    "--state",
                    state,
                    "--timestamp",
                    timestamp,
                    "--source",
                    "routed console",
                    "--ack-time",
                    "time:13,27,39,51",
                    "--allow-write",
                ],
            )
            .await,
            error,
        );
        assert_eq!(alarm_snapshot(&*server.database().read().await), before);
    }
    for service in ["ack-alarm", "ack"] {
        success(
            &invoke(
                port,
                target,
                service,
                &[
                    "ai:77",
                    "--state",
                    "3",
                    "--timestamp",
                    "sequence:42",
                    "--source",
                    "routed console",
                    "--ack-time",
                    "time:13,27,39,51",
                    "--allow-write",
                ],
            )
            .await,
            "Alarm acknowledged",
        );
        let after = alarm_snapshot(&*server.database().read().await);
        assert_eq!(after.0, before.0, "acknowledgment changed event history");
        assert_eq!(after.1, EventTransitionBits::all());
    }
}

#[tokio::test]
async fn routed_object_create_and_delete_change_only_the_selected_object() {
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let (mut router, mut server, port) = site_with_database(database()).await;
        object_lifecycle(&server, Some(port), &target, None).await;
        server.stop().await.unwrap();
        router.stop().await.unwrap();
    }
}

#[tokio::test]
async fn routed_alarm_ack_correlates_exact_history_and_is_idempotent() {
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let (mut router, mut server, port) = site_with_database(database()).await;
        acknowledge(&server, Some(port), &target).await;
        server.stop().await.unwrap();
        router.stop().await.unwrap();
    }
}

#[tokio::test]
async fn direct_object_and_alarm_commands_preserve_output_and_state_changes() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let config = bacnet_server::server::ServerConfig {
        device: lab_identity(INSTANCE),
        ..Default::default()
    };
    let mut server = BACnetServer::start(config, database(), transport)
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    object_lifecycle(&server, None, &target, None).await;
    acknowledge(&server, None, &target).await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn routed_object_and_ack_guards_refuse_before_foreign_registration() {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket.set_nonblocking(true).unwrap();
    let port = socket.local_addr().unwrap().port();
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        for (service, args) in [
            ("create-object", vec!["bi:100"]),
            ("delete-object", vec!["bi:100"]),
            (
                "ack-alarm",
                vec![
                    "ai:77",
                    "--state",
                    "3",
                    "--timestamp",
                    "sequence:42",
                    "--ack-time",
                    "sequence:99",
                ],
            ),
            (
                "ack",
                vec![
                    "ai:77",
                    "--state",
                    "3",
                    "--timestamp",
                    "sequence:42",
                    "--ack-time",
                    "sequence:99",
                ],
            ),
        ] {
            let output = invoke(Some(port), &target, service, &args).await;
            assert_eq!(output.status.code(), Some(4));
            assert!(output.stdout.is_empty());
            let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(refusal["reason"], "allow-write-required");
            assert_eq!(refusal["class"], "write");
            assert_eq!(refusal["missing"], json!(["--allow-write"]));
            assert_eq!(
                refusal["command"],
                if service == "ack" {
                    "ack-alarm"
                } else {
                    service
                }
            );
            assert_eq!(
                socket.recv_from(&mut [0; 1500]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}
