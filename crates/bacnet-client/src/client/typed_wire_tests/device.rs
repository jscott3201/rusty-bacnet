//! Device management, objects, lists, files and alarm acknowledgement.

use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::file::{
    AtomicReadFileAck, AtomicWriteFileAck, FileAccessMethod, FileReadAckMethod,
    FileWriteAccessMethod, FileWriteAckMethod,
};
use bacnet_services::object_mgmt::{CreateObjectAck, ObjectSpecifier};
use bacnet_types::enums::{EnableDisable, EventState, ObjectType, ReinitializedState};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

use super::*;

#[tokio::test]
async fn device_communication_control() {
    let off = EnableDisable::DISABLE_INITIATION;
    let pw = || Some("secret".to_string());
    pin!("dcc/local", Answer::Simple, |c| c
        .device_communication_control(PEER, off, Some(5), pw()));
    pin!("dcc/local", Answer::Simple, |c| c
        .device_communication_control(
            &c.resolve_device(LOCAL).await.unwrap(),
            off,
            Some(5),
            pw()
        ));
    pin!("dcc/routed", Answer::Simple, |c| c
        .device_communication_control(
            &c.resolve_device(REMOTE).await.unwrap(),
            off,
            Some(5),
            pw()
        ));
}

#[tokio::test]
async fn reinitialize_device() {
    let warm = ReinitializedState::WARMSTART;
    let pw = || Some("secret".to_string());
    pin!("reinit/local", Answer::Simple, |c| c.reinitialize_device(
        PEER,
        warm,
        pw()
    ));
    pin!("reinit/local", Answer::Simple, |c| c.reinitialize_device(
        &c.resolve_device(LOCAL).await.unwrap(),
        warm,
        pw()
    ));
    pin!("reinit/routed", Answer::Simple, |c| c.reinitialize_device(
        &c.resolve_device(REMOTE).await.unwrap(),
        warm,
        pw()
    ));
    pin!("reinit-error/local", Answer::Error, |c| c
        .reinitialize_device(PEER, warm, None));
}

const DATE: Date = Date {
    year: 126,
    month: 11,
    day: 15,
    day_of_week: 7,
};
const TIME: Time = Time {
    hour: 12,
    minute: 30,
    second: 5,
    hundredths: 0,
};

#[tokio::test]
async fn time_synchronization() {
    pin!("ts/local", |c| c.time_synchronization(PEER, DATE, TIME));
    pin!("ts/local", |c| c.time_synchronization(
        &c.resolve_device(LOCAL).await.unwrap(),
        DATE,
        TIME
    ));
    pin!("ts/routed", |c| c.time_synchronization(
        &c.resolve_device(REMOTE).await.unwrap(),
        DATE,
        TIME
    ));
    pin!("uts/local", |c| c
        .utc_time_synchronization(PEER, DATE, TIME));
    pin!("uts/local", |c| c.utc_time_synchronization(
        &c.resolve_device(LOCAL).await.unwrap(),
        DATE,
        TIME
    ));
    pin!("uts/routed", |c| c.utc_time_synchronization(
        &c.resolve_device(REMOTE).await.unwrap(),
        DATE,
        TIME
    ));
}

#[tokio::test]
async fn create_and_delete_object() {
    let av5 = oid(ObjectType::ANALOG_VALUE, 5);
    let created = Answer::Complex(octets(|buf| {
        CreateObjectAck {
            object_identifier: av5,
        }
        .encode(buf)
    }));
    let by_type = || ObjectSpecifier::Type(ObjectType::ANALOG_VALUE);
    pin!("create/local", created.clone(), |c| c.create_object(
        PEER,
        by_type(),
        Vec::new()
    ));
    pin!("create/local", created.clone(), |c| c.create_object(
        &c.resolve_device(LOCAL).await.unwrap(),
        by_type(),
        Vec::new()
    ));
    pin!("create/routed", created, |c| c.create_object(
        &c.resolve_device(REMOTE).await.unwrap(),
        by_type(),
        Vec::new()
    ));
    pin!("delete/local", Answer::Simple, |c| c
        .delete_object(PEER, av5));
    pin!("delete/local", Answer::Simple, |c| c
        .delete_object(&c.resolve_device(LOCAL).await.unwrap(), av5));
    pin!("delete/routed", Answer::Simple, |c| c
        .delete_object(&c.resolve_device(REMOTE).await.unwrap(), av5));
}

fn acknowledgement() -> AcknowledgeAlarmRequest {
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 7,
        event_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        event_state_acknowledged: EventState::OFFNORMAL,
        timestamp: BACnetTimeStamp::SequenceNumber(41),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(42),
    }
}

#[tokio::test]
async fn acknowledge_alarm() {
    let ack = acknowledgement();
    pin!("ack-alarm/local", Answer::Simple, |c| c
        .acknowledge_alarm(PEER, &ack));
    pin!("ack-alarm/local", Answer::Simple, |c| c.acknowledge_alarm(
        &c.resolve_device(LOCAL).await.unwrap(),
        &ack
    ));
    pin!("ack-alarm/routed", Answer::Simple, |c| c.acknowledge_alarm(
        &c.resolve_device(REMOTE).await.unwrap(),
        &ack
    ));
}

#[tokio::test]
async fn list_elements() {
    let group = oid(ObjectType::GLOBAL_GROUP, 1);
    let members = PropertyIdentifier::GROUP_MEMBERS;
    let element = || vec![0x21, 0x07];
    pin!("add-list/local", Answer::Simple, |c| c.add_list_element(
        PEER,
        group,
        members,
        None,
        element()
    ));
    pin!("remove-list/local", Answer::Error, |c| c
        .remove_list_element(
            PEER,
            group,
            members,
            Some(1),
            element()
        ));
}

fn file() -> ObjectIdentifier {
    oid(ObjectType::FILE, 2)
}

fn window() -> FileAccessMethod {
    FileAccessMethod::Stream {
        file_start_position: 4,
        requested_octet_count: 3,
    }
}

/// An AtomicReadFile-ACK of `data` from octet 4, at the end of the file.
fn read_ack(data: &[u8]) -> Answer {
    Answer::Complex(octets(|buf| {
        AtomicReadFileAck {
            end_of_file: true,
            access: FileReadAckMethod::Stream {
                file_start_position: 4,
                file_data: data.to_vec(),
            },
        }
        .encode(buf)
    }))
}

#[tokio::test]
async fn atomic_read_file() {
    let ack = read_ack(&[1, 2, 3]);
    pin!("arf/local", ack.clone(), |c| c.atomic_read_file(
        PEER,
        file(),
        window()
    ));
    pin!("arf/local", ack.clone(), |c| c.atomic_read_file(
        &c.resolve_device(LOCAL).await.unwrap(),
        file(),
        window()
    ));
    pin!("arf/routed", ack.clone(), |c| c.atomic_read_file(
        &c.resolve_device(REMOTE).await.unwrap(),
        file(),
        window()
    ));
    pin!("arf-decoded/local", ack.clone(), |c| c
        .atomic_read_file_decoded(PEER, file(), window()));
    pin!("arf-decoded/local", ack.clone(), |c| c
        .atomic_read_file_decoded(
            &c.resolve_device(LOCAL).await.unwrap(),
            file(),
            window()
        ));
    pin!("arf-decoded/routed", ack, |c| c.atomic_read_file_decoded(
        &c.resolve_device(REMOTE).await.unwrap(),
        file(),
        window()
    ));
}

/// Four octets for a window of three: the decoded read refuses them every
/// way, and the raw one returns them.
#[tokio::test]
async fn atomic_read_file_answer_is_checked() {
    let over = read_ack(&[1, 2, 3, 4]);
    pin!("arf-over/local", over.clone(), |c| c.atomic_read_file(
        PEER,
        file(),
        window()
    ));
    pin!("arf-decoded-over/local", over.clone(), |c| c
        .atomic_read_file_decoded(PEER, file(), window()));
    pin!("arf-decoded-over/routed", over, |c| c
        .atomic_read_file_decoded(
            &c.resolve_device(REMOTE).await.unwrap(),
            file(),
            window()
        ));
}

#[tokio::test]
async fn atomic_write_file() {
    let ack = Answer::Complex(octets(|buf| {
        AtomicWriteFileAck {
            access: FileWriteAckMethod::Stream {
                file_start_position: 4,
            },
        }
        .encode(buf)
    }));
    let data = || FileWriteAccessMethod::Stream {
        file_start_position: 4,
        file_data: vec![9, 8, 7],
    };
    pin!("awf/local", ack.clone(), |c| c.atomic_write_file(
        PEER,
        file(),
        data()
    ));
    pin!("awf/local", ack.clone(), |c| c.atomic_write_file(
        &c.resolve_device(LOCAL).await.unwrap(),
        file(),
        data()
    ));
    pin!("awf/routed", ack, |c| c.atomic_write_file(
        &c.resolve_device(REMOTE).await.unwrap(),
        file(),
        data()
    ));
}
