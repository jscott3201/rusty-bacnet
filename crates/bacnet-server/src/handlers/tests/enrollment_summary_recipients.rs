use bacnet_objects::event::EventTransition;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryRequest, RecipientProcess};
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::EventType;
use bacnet_types::MacAddr;

use super::enrollment_summary_support::*;
use super::*;

fn candidate() -> SummaryFixture {
    SummaryFixture::candidate(
        1,
        EventType::OUT_OF_RANGE,
        EventState::OFFNORMAL,
        0b111,
        7,
        Some(EventTransition::ToOffnormal),
    )
}

fn filtered(recipient: BACnetRecipient, process_identifier: u32) -> GetEnrollmentSummaryRequest {
    GetEnrollmentSummaryRequest {
        enrollment_filter: Some(RecipientProcess {
            recipient,
            process_identifier,
        }),
        ..request()
    }
}

#[test]
fn enrollment_membership_matches_exact_device_or_address_and_process() {
    let device = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 44).unwrap());
    let address = BACnetRecipient::Address(BACnetAddress {
        network_number: 5,
        mac_address: MacAddr::from_slice(&[192, 0, 2, 9, 0xba, 0xc0]),
    });
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate())).unwrap();
    db.add(Box::new(class(
        7,
        7,
        [1, 2, 3],
        vec![
            destination(device.clone(), 8),
            destination(address.clone(), 9),
        ],
    )))
    .unwrap();

    assert_eq!(
        response(&db, &filtered(device.clone(), 8))
            .unwrap()
            .entries
            .len(),
        1
    );
    assert_eq!(
        response(&db, &filtered(address.clone(), 9))
            .unwrap()
            .entries
            .len(),
        1
    );
    assert!(response(&db, &filtered(device, 9))
        .unwrap()
        .entries
        .is_empty());
    assert!(response(&db, &filtered(address, 8))
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn membership_ignores_days_time_transitions_and_confirmed_setting() {
    let recipient = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 44).unwrap());
    let configured = destination(recipient.clone(), 8);
    assert_eq!(configured.valid_days, 0);
    assert_eq!(configured.transitions, 0);
    assert!(!configured.issue_confirmed_notifications);
    assert!(configured.to_time.hour < configured.from_time.hour);

    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate())).unwrap();
    db.add(Box::new(class(7, 7, [1, 2, 3], vec![configured])))
        .unwrap();

    assert_eq!(
        response(&db, &filtered(recipient, 8))
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn malformed_recipient_list_matters_only_when_enrollment_filter_is_present() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate())).unwrap();
    db.add(Box::new(SummaryFixture::notification_class(
        7,
        None,
        Some(PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ])),
        Some(PropertyValue::ApplicationData(vec![0x5e])),
    )))
    .unwrap();

    assert_eq!(response(&db, &request()).unwrap().entries.len(), 1);
    let recipient = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 44).unwrap());
    assert_operational_problem(response(&db, &filtered(recipient, 8)).unwrap_err());
}

#[test]
fn unreadable_recipient_list_matters_only_when_enrollment_filter_is_present() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate())).unwrap();
    db.add(Box::new(SummaryFixture::notification_class(
        7,
        None,
        Some(PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ])),
        None,
    )))
    .unwrap();

    assert_eq!(response(&db, &request()).unwrap().entries.len(), 1);
    let recipient = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 44).unwrap());
    assert_operational_problem(response(&db, &filtered(recipient, 8)).unwrap_err());
}
