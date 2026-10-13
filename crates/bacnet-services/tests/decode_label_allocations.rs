//! A decode that succeeds builds no context label (#1645).
//!
//! The decoders name the member they are reading in any error they return,
//! such as `WhoIs low limit` or `AtomicWriteFile record data[1]`. A label made
//! of parts used to be built with `format!` before each member was read, so
//! every decode paid an allocation and a formatting pass per label even when
//! it succeeded, and a `no_std` image linked the formatting for it. Now the
//! labels are written out whole, or, for a record's index, formatted only
//! when an error is.
//!
//! A counting global allocator, one per test binary, so this file is a binary
//! of its own, counts each thread's allocations. Each case decodes a valid
//! encoding and must allocate exactly what the decoded value holds: one
//! buffer per String, Vec or record, plus the copy a BIT STRING's decode
//! makes, and nothing for a label. Each optional member is there in one
//! case, so a label built in its branch would show. The other tests check
//! each label in an error that names it, so its text is what it was.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Debug;

use bacnet_encoding::constructed::encode_audit_notification;
use bacnet_services::audit::{
    AuditLogQueryRequest, BACnetAuditLogQueryParameters, BACnetAuditNotification,
};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicWriteFileRequest, FileReadAckMethod, FileWriteAccessMethod,
};
use bacnet_services::list_manipulation::ChangeListError;
use bacnet_services::object_mgmt::CreateObjectError;
use bacnet_services::private_transfer::{PrivateTransferAck, PrivateTransferRequest};
use bacnet_services::read_range::{RangeSpec, ReadRangeRequest};
use bacnet_services::virtual_terminal::VTDataRequest;
use bacnet_services::who_am_i::{WhoAmIRequest, YouAreRequest};
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{
    AuditOperation, BACnetSuccessFilter, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bacnet_types::MacAddr;
use bytes::BytesMut;

std::thread_local! {
    static THREAD_ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Counts every allocation on the thread that makes it. `realloc` and
/// `alloc_zeroed` keep their default bodies, which call `alloc`, so a buffer
/// that grows counts once per growth.
struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        THREAD_ALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
        // SAFETY: forwarded unchanged to the platform allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to the platform allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Run `f`; return what it returned and the allocations it made on this
/// thread.
fn counted<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = THREAD_ALLOCATIONS.with(Cell::get);
    let value = f();
    (value, THREAD_ALLOCATIONS.with(Cell::get) - before)
}

/// The allocations `decode` makes reading `value`'s encoding by `encode`,
/// after checking that it reads `value` back.
fn decode_allocations<T: PartialEq + Debug>(
    value: &T,
    encode: impl FnOnce(&T, &mut BytesMut),
    decode: impl FnOnce(&[u8]) -> Result<T, Error>,
) -> usize {
    let mut wire = BytesMut::new();
    encode(value, &mut wire);
    let (decoded, allocations) = counted(|| decode(&wire));
    assert_eq!(&decoded.expect("the encoding decodes"), value);
    allocations
}

/// `wire` with its first run of `from` replaced by `to`.
fn patched(wire: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = wire
        .windows(from.len())
        .position(|window| window == from)
        .unwrap_or_else(|| panic!("{from:02X?} not in {wire:02X?}"));
    [&wire[..at], to, &wire[at + from.len()..]].concat()
}

/// The text of the error `decode` refuses `wire` with.
fn refusal<T: Debug>(wire: &[u8], decode: impl FnOnce(&[u8]) -> Result<T, Error>) -> String {
    decode(wire).expect_err("the wire is malformed").to_string()
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn who_is() -> WhoIsRequest {
    WhoIsRequest {
        range: Some(DeviceInstanceRange::new(1, 10).unwrap()),
    }
}

fn who_has() -> WhoHasRequest {
    WhoHasRequest {
        range: Some(DeviceInstanceRange::new(1, 10).unwrap()),
        object: WhoHasObject::Identifier(oid(ObjectType::ANALOG_INPUT, 3)),
    }
}

fn who_am_i() -> WhoAmIRequest {
    WhoAmIRequest {
        vendor_id: 260,
        model_name: "M".into(),
        serial_number: "S".into(),
    }
}

fn you_are() -> YouAreRequest {
    YouAreRequest {
        vendor_id: 260,
        model_name: "M".into(),
        serial_number: "S".into(),
        device_identifier: Some(oid(ObjectType::DEVICE, 7)),
        device_mac_address: Some(vec![1, 2, 3, 4, 5, 6]),
    }
}

fn private_transfer() -> PrivateTransferRequest {
    PrivateTransferRequest {
        vendor_id: 555,
        service_number: 7,
        service_parameters: Some(vec![0x21, 0x05]),
    }
}

fn private_transfer_ack() -> PrivateTransferAck {
    PrivateTransferAck {
        vendor_id: 555,
        service_number: 7,
        result_block: Some(vec![0x21, 0x05]),
    }
}

fn records() -> Vec<Vec<u8>> {
    vec![b"ab".to_vec(), b"cd".to_vec(), b"ef".to_vec()]
}

fn write_file() -> AtomicWriteFileRequest {
    AtomicWriteFileRequest {
        file_identifier: oid(ObjectType::FILE, 1),
        access: FileWriteAccessMethod::Record {
            file_start_record: 0,
            record_count: 3,
            file_record_data: records(),
        },
    }
}

fn read_file_ack() -> AtomicReadFileAck {
    AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Record {
            file_start_record: 0,
            returned_record_count: 3,
            file_record_data: records(),
        },
    }
}

fn change_list_error() -> ChangeListError {
    ChangeListError {
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::UNKNOWN_OBJECT,
        first_failed_element_number: 3,
    }
}

fn create_object_error() -> CreateObjectError {
    CreateObjectError {
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::UNKNOWN_OBJECT,
        first_failed_element_number: 3,
    }
}

/// An AuditLogQuery filtering by target and by source, each on network 7
/// and a two-octet MAC, which fits the address's inline storage. With
/// `every_member`, each optional member the choice has is there too.
fn audit_log_queries(every_member: bool) -> [(&'static str, AuditLogQueryRequest); 2] {
    fn when<T>(on: bool, value: T) -> Option<T> {
        on.then_some(value)
    }
    let operations = || {
        when(
            every_member,
            AuditOperationFlags::from_bits(0x0101).unwrap(),
        )
    };
    let address = || {
        Some(BACnetAddress {
            network_number: 7,
            mac_address: MacAddr::from_slice(&[0xA5, 0xA5]),
        })
    };
    let query = |query_parameters| AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        query_parameters,
        start_at_sequence_number: when(every_member, 9),
        requested_count: 5,
    };
    [
        (
            "by-target",
            query(BACnetAuditLogQueryParameters::ByTarget {
                target_device_identifier: oid(ObjectType::DEVICE, 2),
                target_device_address: address(),
                target_object_identifier: when(every_member, oid(ObjectType::ANALOG_VALUE, 4)),
                target_property_identifier: when(every_member, PropertyIdentifier::PRESENT_VALUE),
                target_array_index: when(every_member, 2),
                target_priority: when(every_member, 8),
                operations: operations(),
                successful_actions_only: BACnetSuccessFilter::ALL,
            }),
        ),
        (
            "by-source",
            query(BACnetAuditLogQueryParameters::BySource {
                source_device_identifier: oid(ObjectType::DEVICE, 2),
                source_device_address: address(),
                source_object_identifier: when(every_member, oid(ObjectType::ANALOG_VALUE, 4)),
                operations: operations(),
                successful_actions_only: BACnetSuccessFilter::ALL,
            }),
        ),
    ]
}

fn audit_notification(
    target_value: Option<Vec<u8>>,
    current_value: Option<Vec<u8>>,
) -> BACnetAuditNotification {
    let device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 2));
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: device.clone(),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: device,
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value,
        current_value,
        result: None,
    }
}

/// Each case's allocations, as (case, count), all measured before any is
/// compared, so a failure lists every case that changed.
#[test]
fn a_decode_that_succeeds_allocates_only_its_values() {
    let [(_, by_target), (_, by_source)] = audit_log_queries(false);
    let audit_notification_encode = {
        // The encoder checks both raw values, which built a label each. The
        // buffer has room for the encoding, so encoding allocates nothing.
        let notification = audit_notification(Some(vec![0x21, 0x05]), Some(vec![0x21, 0x04]));
        let mut wire = BytesMut::with_capacity(256);
        let (encoded, allocations) =
            counted(|| encode_audit_notification(&notification, &mut wire));
        encoded.expect("the notification encodes");
        allocations
    };
    let measured = [
        (
            "WhoIs",
            decode_allocations(&who_is(), |v, buf| v.encode(buf), WhoIsRequest::decode),
        ),
        (
            "WhoHas",
            decode_allocations(
                &who_has(),
                |v, buf| v.encode(buf).unwrap(),
                WhoHasRequest::decode,
            ),
        ),
        (
            "WhoAmI",
            decode_allocations(
                &who_am_i(),
                |v, buf| v.encode(buf).unwrap(),
                WhoAmIRequest::decode,
            ),
        ),
        (
            "YouAre",
            decode_allocations(
                &you_are(),
                |v, buf| v.encode(buf).unwrap(),
                YouAreRequest::decode,
            ),
        ),
        (
            "PrivateTransfer",
            decode_allocations(
                &private_transfer(),
                |v, buf| v.encode(buf),
                PrivateTransferRequest::decode,
            ),
        ),
        (
            "PrivateTransferAck",
            decode_allocations(
                &private_transfer_ack(),
                |v, buf| v.encode(buf),
                PrivateTransferAck::decode,
            ),
        ),
        (
            "AtomicWriteFile",
            decode_allocations(
                &write_file(),
                |v, buf| v.encode(buf),
                AtomicWriteFileRequest::decode,
            ),
        ),
        (
            "AtomicReadFileAck",
            decode_allocations(
                &read_file_ack(),
                |v, buf| v.encode(buf),
                AtomicReadFileAck::decode,
            ),
        ),
        (
            "ChangeList-Error",
            decode_allocations(
                &change_list_error(),
                |v, buf| v.encode(buf),
                ChangeListError::decode,
            ),
        ),
        (
            "CreateObject-Error",
            decode_allocations(
                &create_object_error(),
                |v, buf| v.encode(buf),
                CreateObjectError::decode,
            ),
        ),
        (
            "AuditLogQuery by-target",
            decode_allocations(
                &by_target,
                |v, buf| v.encode(buf).unwrap(),
                AuditLogQueryRequest::decode,
            ),
        ),
        (
            "AuditLogQuery by-source",
            decode_allocations(
                &by_source,
                |v, buf| v.encode(buf).unwrap(),
                AuditLogQueryRequest::decode,
            ),
        ),
        ("AuditNotification encode", audit_notification_encode),
    ];
    let expected = [
        ("WhoIs", 0),
        ("WhoHas", 0),
        // The model name and the serial number.
        ("WhoAmI", 2),
        // Those, and the MAC address.
        ("YouAre", 3),
        // The block.
        ("PrivateTransfer", 1),
        ("PrivateTransferAck", 1),
        // The list of records, and each of the three.
        ("AtomicWriteFile", 4),
        ("AtomicReadFileAck", 4),
        ("ChangeList-Error", 0),
        ("CreateObject-Error", 0),
        ("AuditLogQuery by-target", 0),
        ("AuditLogQuery by-source", 0),
        ("AuditNotification encode", 0),
    ];
    assert_eq!(measured, expected);
}

/// The members a decoder reads only when they are there, which the values
/// above leave out, and two decoders that name their members in errors the
/// same way, ReadRange's and VT-Data's: each decode allocates only what its
/// value holds, as above.
#[test]
fn optional_members_and_other_decoders_allocate_only_their_values() {
    let [(_, by_target), (_, by_source)] = audit_log_queries(true);
    let you_are_without_mac = YouAreRequest {
        device_mac_address: None,
        ..you_are()
    };
    let read_range = |property_array_index, range| ReadRangeRequest {
        object_identifier: oid(ObjectType::TREND_LOG, 1),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index,
        range: Some(range),
    };
    let by_position = read_range(
        Some(1),
        RangeSpec::ByPosition {
            reference_index: 3,
            count: 5,
        },
    );
    let by_sequence = read_range(
        None,
        RangeSpec::BySequenceNumber {
            reference_seq: 70_000,
            count: -5,
        },
    );
    let by_time = read_range(
        None,
        RangeSpec::ByTime {
            reference_time: (
                Date {
                    year: 126,
                    month: 3,
                    day: 1,
                    day_of_week: 7,
                },
                Time {
                    hour: 14,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
            ),
            count: 10,
        },
    );
    let vt_data = VTDataRequest {
        vt_session_identifier: 5,
        vt_new_data: b"hi".to_vec(),
        vt_data_flag: true,
    };
    let audit_query = |request: &AuditLogQueryRequest| {
        decode_allocations(
            request,
            |v, buf| v.encode(buf).unwrap(),
            AuditLogQueryRequest::decode,
        )
    };
    let read_range = |request: &ReadRangeRequest| {
        decode_allocations(
            request,
            |v, buf| v.encode(buf).unwrap(),
            ReadRangeRequest::decode,
        )
    };
    let measured = [
        (
            "AuditLogQuery by-target, every member",
            audit_query(&by_target),
        ),
        (
            "AuditLogQuery by-source, every member",
            audit_query(&by_source),
        ),
        (
            "YouAre without a MAC",
            decode_allocations(
                &you_are_without_mac,
                |v, buf| v.encode(buf).unwrap(),
                YouAreRequest::decode,
            ),
        ),
        ("ReadRange by position", read_range(&by_position)),
        ("ReadRange by sequence number", read_range(&by_sequence)),
        ("ReadRange by time", read_range(&by_time)),
        (
            "VTData",
            decode_allocations(&vt_data, |v, buf| v.encode(buf), VTDataRequest::decode),
        ),
    ];
    let expected = [
        // The operations' bit string, which the decoder copies out before it
        // reads the flags from it.
        ("AuditLogQuery by-target, every member", 1),
        ("AuditLogQuery by-source, every member", 1),
        // The model name and the serial number.
        ("YouAre without a MAC", 2),
        ("ReadRange by position", 0),
        ("ReadRange by sequence number", 0),
        ("ReadRange by time", 0),
        // The new data.
        ("VTData", 1),
    ];
    assert_eq!(measured, expected);
}

/// A Who-Is's or Who-Has's limit of 2^32, one past what u32 holds.
#[test]
fn limit_labels() {
    let low = [0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00];
    let high = [0x09, 0x01, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00];
    for (service, decode) in [
        (
            "WhoIs",
            (|wire| WhoIsRequest::decode(wire).map(drop)) as fn(&[u8]) -> Result<(), Error>,
        ),
        ("WhoHas", |wire| WhoHasRequest::decode(wire).map(drop)),
    ] {
        assert_eq!(
            refusal(&low, decode),
            format!(
                "decoding error at offset 0: {service} low limit: [0] value 4294967296 exceeds u32"
            ),
        );
        assert_eq!(
            refusal(&high, decode),
            format!("decoding error at offset 2: {service} high limit: [1] value 4294967296 exceeds u32"),
        );
    }
}

/// An ENUMERATED for the vendor; an Unsigned for the model name; an empty
/// model name, then an Unsigned for the serial number.
#[test]
fn identity_labels() {
    let vendor = [0x91, 0x00];
    let model = [0x21, 0x01, 0x21, 0x00];
    let serial = [0x21, 0x01, 0x71, 0x00, 0x21, 0x00];
    for (service, decode) in [
        (
            "WhoAmI",
            (|wire| WhoAmIRequest::decode(wire).map(drop)) as fn(&[u8]) -> Result<(), Error>,
        ),
        ("YouAre", |wire| YouAreRequest::decode(wire).map(drop)),
    ] {
        assert_eq!(
            refusal(&vendor, decode),
            format!("decoding error at offset 0: {service} vendor-id: expected application-tagged Unsigned"),
        );
        assert_eq!(
            refusal(&model, decode),
            format!(
                "decoding error at offset 2: {service} model-name: \
                 expected application-tagged CharacterString"
            ),
        );
        assert_eq!(
            refusal(&serial, decode),
            format!(
                "decoding error at offset 4: {service} serial-number: \
                 expected application-tagged CharacterString"
            ),
        );
    }
}

/// A `[1]` where the `[0]` vendorID is due, then a `[2]` where the `[1]`
/// serviceNumber is. The block's label names no error a decoder returns: it
/// reads the block only once the block's opening tag is there.
#[test]
fn private_transfer_labels() {
    let vendor = [0x19, 0x01];
    let service = [0x09, 0x01, 0x29, 0x01];
    for (what, decode) in [
        (
            "PrivateTransfer",
            (|wire| PrivateTransferRequest::decode(wire).map(drop))
                as fn(&[u8]) -> Result<(), Error>,
        ),
        ("PrivateTransferAck", |wire| {
            PrivateTransferAck::decode(wire).map(drop)
        }),
    ] {
        assert_eq!(
            refusal(&vendor, decode),
            format!("decoding error at offset 0: {what} vendorID: expected context tag [0]"),
        );
        assert_eq!(
            refusal(&service, decode),
            format!("decoding error at offset 2: {what} serviceNumber: expected context tag [1]"),
        );
    }
}

/// The second record as an Unsigned rather than an OCTET STRING.
#[test]
fn record_labels() {
    let mut wire = BytesMut::new();
    write_file().encode(&mut wire);
    let wire = patched(&wire, &[0x62, b'c', b'd'], &[0x22, b'c', b'd']);
    assert_eq!(
        refusal(&wire, AtomicWriteFileRequest::decode),
        "decoding error at offset 7: AtomicWriteFile record data[1]: \
         expected application-tagged OCTET STRING",
    );

    let mut wire = BytesMut::new();
    read_file_ack().encode(&mut wire);
    let wire = patched(&wire, &[0x62, b'c', b'd'], &[0x22, b'c', b'd']);
    assert_eq!(
        refusal(&wire, AtomicReadFileAck::decode),
        "decoding error at offset 7: AtomicReadFileAck record data[1]: \
         expected application-tagged OCTET STRING",
    );
}

/// A `[2]` where the `[1]` first-failed-element-number is due.
#[test]
fn element_error_labels() {
    let mut wire = BytesMut::new();
    change_list_error().encode(&mut wire);
    let wire = patched(&wire, &[0x19, 0x03], &[0x29, 0x03]);
    assert_eq!(
        refusal(&wire, ChangeListError::decode),
        "decoding error at offset 6: ChangeList-Error first-failed-element-number: \
         expected context tag [1]",
    );
    assert_eq!(
        refusal(&wire, CreateObjectError::decode),
        "decoding error at offset 6: CreateObject-Error first-failed-element-number: \
         expected context tag [1]",
    );
}

/// The address's network number as an ENUMERATED, then its MAC as an
/// Unsigned.
#[test]
fn audit_address_labels() {
    for (choice, request) in audit_log_queries(false) {
        let mut wire = BytesMut::new();
        request.encode(&mut wire).unwrap();
        let network = patched(&wire, &[0x21, 0x07, 0x62], &[0x91, 0x07, 0x62]);
        assert_eq!(
            refusal(&network, AuditLogQueryRequest::decode),
            format!(
                "decoding error at offset 0: AuditLogQuery {choice} address network-number: \
                 expected application-tagged Unsigned"
            ),
        );
        let mac = patched(&wire, &[0x21, 0x07, 0x62], &[0x21, 0x07, 0x22]);
        assert_eq!(
            refusal(&mac, AuditLogQueryRequest::decode),
            format!(
                "decoding error at offset 2: AuditLogQuery {choice} address mac-address: \
                 expected application-tagged OCTET STRING"
            ),
        );
    }
}

/// A raw value of one more NULL than a value may hold.
#[test]
fn audit_value_labels() {
    let long = || Some(vec![0x00; 10_001]);
    let refused = |notification: BACnetAuditNotification| {
        encode_audit_notification(&notification, &mut BytesMut::new())
            .expect_err("the value is too long")
            .to_string()
    };
    for (field, notification) in [
        ("target-value", audit_notification(long(), None)),
        ("current-value", audit_notification(None, long())),
    ] {
        assert_eq!(
            refused(notification),
            format!(
                "encoding error: decoding error at offset 10000: AuditNotification {field}: \
                 sequence exceeds item limit"
            ),
        );
    }
}
