//! Every way of naming a station meets the pins (#2588): a MAC as a slice,
//! an array, a `Vec` or a `MacAddr`, an address on network 0, and a local
//! `Station`, for the device on this link; an address the discovery table
//! routes, a remote `Station` and a `Station` through a named router, for
//! the one behind the router. Services that had no routed form before have
//! routed pins of their own, which every routed form must meet alike.

use std::num::NonZeroU32;

use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::audit::{
    AuditLogQueryRequest, AuditNotificationRequest, BACnetAuditLogQueryParameters,
};
use bacnet_services::enrollment_summary::GetEnrollmentSummaryRequest;
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{
    AcknowledgmentFilter, BACnetSuccessFilter, EnableDisable, EventState, ObjectType,
    ReinitializedState,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

use super::*;
use crate::client::cov::CovPropertySubscription;
use crate::client::station::Station;
use crate::log_reader::LogCursor;

fn address(network: u16, mac: &[u8]) -> BACnetAddress {
    BACnetAddress {
        network_number: network,
        mac_address: MacAddr::from_slice(mac),
    }
}

/// Pin `$call` with each local form of the station as `$to` against
/// `$local`, and each routed form against `$routed`; `$answer` answers each
/// confirmed request, and `unconfirmed` marks a call that has no answer.
macro_rules! every_form {
    ($local:expr, $routed:expr, unconfirmed, |$c:ident, $to:ident| $call:expr) => {
        every_form!(@ $local, $routed, [], $c, $to, $call)
    };
    ($local:expr, $routed:expr, $answer:expr, |$c:ident, $to:ident| $call:expr) => {
        every_form!(@ $local, $routed, [$answer], $c, $to, $call)
    };
    (@ $local:expr, $routed:expr, [$($answer:expr)?], $c:ident, $to:ident, $call:expr) => {{
        let array: [u8; 1] = [PEER[0]];
        let vec = PEER.to_vec();
        let mac = MacAddr::from_slice(PEER);
        let at_zero = address(0, PEER);
        let local = Station::local(PEER);
        {
            let $to = PEER;
            pin!($local, $($answer,)? |$c| $call);
        }
        {
            let $to = &array;
            pin!($local, $($answer,)? |$c| $call);
        }
        {
            let $to = &vec;
            pin!($local, $($answer,)? |$c| $call);
        }
        {
            let $to = &mac;
            pin!($local, $($answer,)? |$c| $call);
        }
        {
            let $to = &at_zero;
            pin!($local, $($answer,)? |$c| $call);
        }
        {
            let $to = &local;
            pin!($local, $($answer,)? |$c| $call);
        }
        let learned = address(DNET, DADR);
        let remote = Station::remote(DNET, DADR);
        let routed = Station::routed(ROUTER, DNET, DADR);
        {
            let $to = &learned;
            pin!($routed, $($answer,)? |$c| $call);
        }
        {
            let $to = &remote;
            pin!($routed, $($answer,)? |$c| $call);
        }
        {
            let $to = &routed;
            pin!($routed, $($answer,)? |$c| $call);
        }
    }};
}

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

/// The service octets of a ReadProperty-ACK for `object`'s present value.
fn rp_ack(object: ObjectIdentifier) -> Answer {
    Answer::Complex(octets(|buf| {
        bacnet_services::read_property::ReadPropertyACK {
            object_identifier: object,
            property_identifier: PV,
            property_array_index: None,
            property_value: vec![0x44, 0x41, 0xA8, 0x00, 0x00],
        }
        .encode(buf)
        .unwrap()
    }))
}

#[tokio::test]
async fn reads_and_writes() {
    every_form!("rp/local", "rp/routed", rp_ack(ai1()), |c, to| c
        .read_property(to, ai1(), PV, None));
    let other = || rp_ack(oid(ObjectType::ANALOG_INPUT, 2));
    every_form!(
        "rp-mismatch/local",
        "rp-mismatch/routed",
        other(),
        |c, to| c.read_property(to, ai1(), PV, None)
    );
    let ao1 = oid(ObjectType::ANALOG_OUTPUT, 1);
    let value = || vec![0x44, 0x42, 0x91, 0x00, 0x00];
    every_form!("wp/local", "wp/routed", Answer::Simple, |c, to| c
        .write_property(to, ao1, PV, None, value(), Some(8)));
    let wp = ConfirmedServiceChoice::WRITE_PROPERTY;
    let data = [0x0C, 0x00, 0x00, 0x00, 0x01];
    every_form!(
        "generic/local",
        "generic/routed",
        Answer::Simple,
        |c, to| c.confirmed_request(to, wp, &data)
    );
    let ts = UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;
    let data = [0xA4, 0x7E, 0x0B, 0x0F, 0x07, 0xB4, 0x0C, 0x00, 0x00, 0x00];
    every_form!("ugeneric/local", "ugeneric/routed", unconfirmed, |c, to| c
        .unconfirmed_request(to, ts, &data));
}

#[tokio::test]
async fn multiple_reads_and_writes() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
    use bacnet_services::wpm::WriteAccessSpecification;
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    let specs = || {
        vec![ReadAccessSpecification {
            object_identifier: ai1(),
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PV,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                    property_array_index: Some(3),
                },
            ],
        }]
    };
    let ack = || {
        Answer::Complex(octets(|buf| {
            ReadPropertyMultipleACK {
                list_of_read_access_results: vec![ReadAccessResult {
                    object_identifier: ai1(),
                    list_of_results: vec![ReadResultElement {
                        property_identifier: PV,
                        property_array_index: None,
                        property_value: Some(vec![0x44, 0x41, 0xA8, 0x00, 0x00]),
                        error: None,
                    }],
                }],
            }
            .encode(buf)
            .unwrap()
        }))
    };
    every_form!("rpm/local", "rpm/routed", ack(), |c, to| c
        .read_property_multiple(to, specs()));
    let writes = || {
        vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::ANALOG_VALUE, 3),
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PV,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x91, 0x00, 0x00],
                priority: Some(10),
            }],
        }]
    };
    every_form!("wpm/local", "wpm/routed", Answer::Simple, |c, to| c
        .write_property_multiple(to, writes()));
}

#[tokio::test]
async fn ranges_and_log_pages() {
    use bacnet_services::read_range::ReadRangeValidation::{Lenient, Strict};
    use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
    let log = oid(ObjectType::TREND_LOG, 1);
    let buffer = PropertyIdentifier::LOG_BUFFER;
    let range = || {
        Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 2,
        })
    };
    let ack = |items: u32| {
        Answer::Complex(octets(|buf| {
            ReadRangeAck {
                object_identifier: log,
                property_identifier: buffer,
                property_array_index: None,
                result_flags: (true, true, false),
                item_count: items,
                item_data: (0..items).flat_map(|_| [0x0E, 0x0F]).collect(),
                first_sequence_number: None,
            }
            .encode(buf)
        }))
    };
    every_form!("rr/local", "rr/routed", ack(2), |c, to| c.read_range(
        to,
        log,
        buffer,
        None,
        range()
    ));
    let request = ReadRangeRequest {
        object_identifier: log,
        property_identifier: buffer,
        property_array_index: None,
        range: range(),
    };
    every_form!("rr-strict/local", "rr-strict/routed", ack(3), |c, to| c
        .read_range_with(to, &request, Strict));
    every_form!("rr-lenient/local", "rr-lenient/routed", ack(3), |c, to| c
        .read_range_with(to, &request, Lenient));
    let from = LogCursor::Sequence(5);
    every_form!("log/local", "log/routed", Answer::Error, |c, to| c
        .read_log_page(to, log, from, 2));
}

#[tokio::test]
async fn device_management() {
    let off = EnableDisable::DISABLE_INITIATION;
    let pw = || Some("secret".to_string());
    every_form!("dcc/local", "dcc/routed", Answer::Simple, |c, to| c
        .device_communication_control(to, off, Some(5), pw()));
    let warm = ReinitializedState::WARMSTART;
    every_form!("reinit/local", "reinit/routed", Answer::Simple, |c, to| c
        .reinitialize_device(to, warm, pw()));
    let date = Date {
        year: 126,
        month: 11,
        day: 15,
        day_of_week: 7,
    };
    let time = Time {
        hour: 12,
        minute: 30,
        second: 5,
        hundredths: 0,
    };
    every_form!("ts/local", "ts/routed", unconfirmed, |c, to| c
        .time_synchronization(to, date, time));
    every_form!("uts/local", "uts/routed", unconfirmed, |c, to| c
        .utc_time_synchronization(to, date, time));
    every_form!("who-is/local", "who-is/routed", unconfirmed, |c, to| c
        .who_is_directed(to, None));
}

#[tokio::test]
async fn objects_and_lists() {
    let av5 = oid(ObjectType::ANALOG_VALUE, 5);
    let created = || {
        Answer::Complex(octets(|buf| {
            bacnet_services::object_mgmt::CreateObjectAck {
                object_identifier: av5,
            }
            .encode(buf)
        }))
    };
    let by_type = || ObjectSpecifier::Type(ObjectType::ANALOG_VALUE);
    every_form!("create/local", "create/routed", created(), |c, to| c
        .create_object(to, by_type(), Vec::new()));
    every_form!("delete/local", "delete/routed", Answer::Simple, |c, to| c
        .delete_object(to, av5));
    let group = oid(ObjectType::GLOBAL_GROUP, 1);
    let members = PropertyIdentifier::GROUP_MEMBERS;
    let element = || vec![0x21, 0x07];
    every_form!(
        "add-list/local",
        "add-list/routed",
        Answer::Simple,
        |c, to| c.add_list_element(to, group, members, None, element())
    );
    every_form!(
        "remove-list/local",
        "remove-list/routed",
        Answer::Error,
        |c, to| c.remove_list_element(to, group, members, Some(1), element())
    );
}

#[tokio::test]
async fn files() {
    use bacnet_services::file::{
        AtomicReadFileAck, AtomicWriteFileAck, FileReadAckMethod, FileWriteAckMethod,
    };
    let file = oid(ObjectType::FILE, 2);
    let window = || FileAccessMethod::Stream {
        file_start_position: 4,
        requested_octet_count: 3,
    };
    let read = |data: &[u8]| {
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
    };
    every_form!("arf/local", "arf/routed", read(&[1, 2, 3]), |c, to| c
        .atomic_read_file(to, file, window()));
    every_form!(
        "arf-decoded/local",
        "arf-decoded/routed",
        read(&[1, 2, 3]),
        |c, to| c.atomic_read_file_decoded(to, file, window())
    );
    every_form!(
        "arf-decoded-over/local",
        "arf-decoded-over/routed",
        read(&[1, 2, 3, 4]),
        |c, to| c.atomic_read_file_decoded(to, file, window())
    );
    let written = || {
        Answer::Complex(octets(|buf| {
            AtomicWriteFileAck {
                access: FileWriteAckMethod::Stream {
                    file_start_position: 4,
                },
            }
            .encode(buf)
        }))
    };
    let data = || FileWriteAccessMethod::Stream {
        file_start_position: 4,
        file_data: vec![9, 8, 7],
    };
    every_form!("awf/local", "awf/routed", written(), |c, to| c
        .atomic_write_file(to, file, data()));
}

#[tokio::test]
async fn alarms_and_events() {
    use bacnet_services::alarm_event::GetEventInformationAck;
    use bacnet_services::alarm_summary::GetAlarmSummaryAck;
    use bacnet_services::enrollment_summary::GetEnrollmentSummaryAck;
    use bacnet_types::enums::EnrollmentSummaryEventStateFilter;
    let ack = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 7,
        event_object_identifier: ai1(),
        event_state_acknowledged: EventState::OFFNORMAL,
        timestamp: BACnetTimeStamp::SequenceNumber(41),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(42),
    };
    every_form!(
        "ack-alarm/local",
        "ack-alarm/routed",
        Answer::Simple,
        |c, to| c.acknowledge_alarm(to, &ack)
    );
    let information = || {
        Answer::Complex(octets(|buf| {
            GetEventInformationAck {
                list_of_event_summaries: Vec::new(),
                more_events: false,
            }
            .encode(buf)
            .unwrap()
        }))
    };
    let after = Some(oid(ObjectType::BINARY_INPUT, 3));
    every_form!("gei/local", "gei/routed", information(), |c, to| c
        .get_event_information(to, after));
    let alarms = || {
        Answer::Complex(octets(|buf| {
            GetAlarmSummaryAck {
                entries: Vec::new(),
            }
            .encode(buf)
        }))
    };
    every_form!("gas/local", "gas/routed", alarms(), |c, to| c
        .get_alarm_summary(to));
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: AcknowledgmentFilter::NOT_ACKED,
        enrollment_filter: None,
        event_state_filter: Some(EnrollmentSummaryEventStateFilter::OFFNORMAL),
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: Some(4),
    };
    let enrollments = || {
        Answer::Complex(octets(|buf| {
            GetEnrollmentSummaryAck {
                entries: Vec::new(),
            }
            .encode(buf)
        }))
    };
    every_form!("ges/local", "ges/routed", enrollments(), |c, to| c
        .get_enrollment_summary(to, &request));
}

#[tokio::test]
async fn cov_subscriptions() {
    every_form!("cov/local", "cov/routed", Answer::Simple, |c, to| c
        .subscribe_cov(to, 9, ai1(), true, Some(300)));
    every_form!(
        "cov-cancel/local",
        "cov-cancel/routed",
        Answer::Simple,
        |c, to| c.unsubscribe_cov(to, 9, ai1())
    );
    let subscription = || CovPropertySubscription {
        subscriber_process_identifier: 9,
        monitored_object_identifier: ai1(),
        monitored_property_identifier: PV,
        monitored_property_array_index: None,
        confirmed: false,
        lifetime: NonZeroU32::new(120).unwrap(),
        cov_increment: Some(0.5),
    };
    every_form!("covp/local", "covp/routed", Answer::Simple, |c, to| c
        .subscribe_cov_property(to, subscription()));
    every_form!(
        "covp-cancel/local",
        "covp-cancel/routed",
        Answer::Simple,
        |c, to| c.unsubscribe_cov_property(to, 9, ai1(), PV, Some(2))
    );
}

#[tokio::test]
async fn audit() {
    let note: AuditNotificationRequest = super::events::notification();
    every_form!(
        "audit-note/local",
        "audit-note/routed",
        Answer::Simple,
        |c, to| c.confirmed_audit_notification(to, &note)
    );
    every_form!(
        "uaudit-note/local",
        "uaudit-note/routed",
        unconfirmed,
        |c, to| c.unconfirmed_audit_notification(to, &note)
    );
    let query = AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: oid(ObjectType::DEVICE, 1),
            source_device_address: None,
            source_object_identifier: None,
            operations: None,
            successful_actions_only: BACnetSuccessFilter::ALL,
        },
        start_at_sequence_number: None,
        requested_count: 4,
    };
    every_form!(
        "audit-query/local",
        "audit-query/routed",
        Answer::Error,
        |c, to| c.audit_log_query(to, &query)
    );
}
