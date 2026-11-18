//! The pins: case, the frame sent (link MAC, then the NPDU, in hex) and
//! the call's result (its `Debug` text), recorded from the by-MAC and
//! by-instance calls before #2588 moved them onto one service description.
//! Seven are newer, for calls that had no form to record them from: the
//! routed forms of AddListElement, RemoveListElement, the two Audit
//! notifications and AuditLogQuery, and the directed Who-Is both ways.
//! The pins of `remaining.rs` and `scope.rs` were recorded from the octets
//! the bindings and ops encoded and the answers they decoded, sent through
//! `confirmed_request`, `unconfirmed_request` and the broadcast calls,
//! before those services had typed calls.
//! The eight of the `forms_without_*` tests were recorded the same way,
//! through those raw sends, which are dev's, after the typed calls existed.
//! The `wg/` pins were recorded from `write_group` to each
//! `WriteGroupDestination` before WriteGroup took a `Broadcast` scope.
//! `wg/routed` is newer, recorded from `write_group_directed`: no
//! destination named a station behind a router.

pub(super) const PINS: &[(&str, &str, &str)] = &[
    (
        "ack-alarm/local",
        "02 01040205000009071c0000000129023e19293f4d09006f70657261746f725e192a5f",
        r#"Ok(())"#,
    ),
    (
        "ack-alarm/routed",
        "09 012400020105ff0204000009071c0000000129023e19293f4d09006f70657261746f725e192a5f",
        r#"Ok(())"#,
    ),
    (
        "add-list/local",
        "02 0104020500080c068000011a01593e21073f",
        r#"Ok(())"#,
    ),
    (
        "add-list/routed",
        "09 012400020105ff020400080c068000011a01593e21073f",
        r#"Ok(())"#,
    ),
    (
        "arf-decoded-over/local",
        "02 010402050006c4028000020e310421030f",
        r#"Err(Decoding(DecodeError { offset: 0, kind: InvalidEncoding, context: "", label: "AtomicReadFile ACK exceeds the requested octet window", values: [0, 0], index: None }))"#,
    ),
    (
        "arf-decoded-over/routed",
        "09 012400020105ff02040006c4028000020e310421030f",
        r#"Err(Decoding(DecodeError { offset: 0, kind: InvalidEncoding, context: "", label: "AtomicReadFile ACK exceeds the requested octet window", values: [0, 0], index: None }))"#,
    ),
    (
        "arf-decoded/local",
        "02 010402050006c4028000020e310421030f",
        r#"Ok(AtomicReadFileAck { end_of_file: true, access: Stream { file_start_position: 4, file_data: [1, 2, 3] } })"#,
    ),
    (
        "arf-decoded/routed",
        "09 012400020105ff02040006c4028000020e310421030f",
        r#"Ok(AtomicReadFileAck { end_of_file: true, access: Stream { file_start_position: 4, file_data: [1, 2, 3] } })"#,
    ),
    (
        "arf-over/local",
        "02 010402050006c4028000020e310421030f",
        r#"Ok(b"\x11\x0e1\x04d\x01\x02\x03\x04\x0f")"#,
    ),
    (
        "arf/local",
        "02 010402050006c4028000020e310421030f",
        r#"Ok(b"\x11\x0e1\x04c\x01\x02\x03\x0f")"#,
    ),
    (
        "arf/routed",
        "09 012400020105ff02040006c4028000020e310421030f",
        r#"Ok(b"\x11\x0e1\x04c\x01\x02\x03\x0f")"#,
    ),
    (
        "audit-note/local",
        "02 0104020500200e2e0c020000012f4901ae0c02000002af0f",
        r#"Ok(())"#,
    ),
    (
        "audit-note/routed",
        "09 012400020105ff020400200e2e0c020000012f4901ae0c02000002af0f",
        r#"Ok(())"#,
    ),
    (
        "audit-query/local",
        "02 0104020500210c0f4000011e1e0c0200000149001f1f3904",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "audit-query/routed",
        "09 012400020105ff020400210c0f4000011e1e0c0200000149001f1f3904",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "awf/local",
        "02 010402050007c4028000020e3104630908070f",
        r#"Ok(b"\t\x04")"#,
    ),
    (
        "awf/routed",
        "09 012400020105ff02040007c4028000020e3104630908070f",
        r#"Ok(b"\t\x04")"#,
    ),
    (
        "cov-cancel/local",
        "02 01040205000509091c00000001",
        r#"Ok(())"#,
    ),
    (
        "cov-cancel/routed",
        "09 012400020105ff0204000509091c00000001",
        r#"Ok(())"#,
    ),
    (
        "cov/local",
        "02 01040205000509091c0000000129013a012c",
        r#"Ok(())"#,
    ),
    (
        "cov/routed",
        "09 012400020105ff0204000509091c0000000129013a012c",
        r#"Ok(())"#,
    ),
    (
        "covp-cancel/local",
        "02 01040205001c09091c000000014e095519024f",
        r#"Ok(())"#,
    ),
    (
        "covp-cancel/routed",
        "09 012400020105ff0204001c09091c000000014e095519024f",
        r#"Ok(())"#,
    ),
    (
        "covp/local",
        "02 01040205001c09091c00000001290039784e09554f5c3f000000",
        r#"Ok(())"#,
    ),
    (
        "covp/routed",
        "09 012400020105ff0204001c09091c00000001290039784e09554f5c3f000000",
        r#"Ok(())"#,
    ),
    (
        "cpt-error/local",
        "02 0104020500120a022b19072e21052f",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "cpt-noparams/local",
        "02 0104020500120a022b1908",
        r#"Ok(PrivateTransferAck { vendor_id: 555, service_number: 8, result_block: None })"#,
    ),
    (
        "cpt/local",
        "02 0104020500120a022b19072e21052f",
        r#"Ok(PrivateTransferAck { vendor_id: 555, service_number: 7, result_block: Some([33, 6]) })"#,
    ),
    (
        "cpt/routed",
        "09 012400020105ff020400120a022b19072e21052f",
        r#"Ok(PrivateTransferAck { vendor_id: 555, service_number: 7, result_block: Some([33, 6]) })"#,
    ),
    (
        "create/local",
        "02 01040205000a0e09020f",
        r#"Ok(ObjectIdentifier(ObjectType::ANALOG_VALUE, 5))"#,
    ),
    (
        "create/routed",
        "09 012400020105ff0204000a0e09020f",
        r#"Ok(ObjectIdentifier(ObjectType::ANALOG_VALUE, 5))"#,
    ),
    (
        "ctm-noclass/local",
        "02 0104020500130c0200000929013d0b00636865636b2070756d70",
        r#"Ok(())"#,
    ),
    (
        "ctm/local",
        "02 0104020500130c020000091e1d06006d61696e741f29013d0b00636865636b2070756d70",
        r#"Ok(())"#,
    ),
    (
        "ctm/routed",
        "09 012400020105ff020400130c020000091e1d06006d61696e741f29013d0b00636865636b2070756d70",
        r#"Ok(())"#,
    ),
    (
        "dcc/local",
        "02 010402050011090519022d0700736563726574",
        r#"Ok(())"#,
    ),
    (
        "dcc/routed",
        "09 012400020105ff02040011090519022d0700736563726574",
        r#"Ok(())"#,
    ),
    ("delete/local", "02 01040205000bc400800005", r#"Ok(())"#),
    (
        "delete/routed",
        "09 012400020105ff0204000bc400800005",
        r#"Ok(())"#,
    ),
    (
        "gas/local",
        "02 010402050003",
        r#"Ok(GetAlarmSummaryAck { entries: [] })"#,
    ),
    (
        "gas/routed",
        "09 012400020105ff02040003",
        r#"Ok(GetAlarmSummaryAck { entries: [] })"#,
    ),
    (
        "gei/local",
        "02 01040205001d0c00c00003",
        r#"Ok(GetEventInformationAck { list_of_event_summaries: [], more_events: false })"#,
    ),
    (
        "gei/routed",
        "09 012400020105ff0204001d0c00c00003",
        r#"Ok(GetEventInformationAck { list_of_event_summaries: [], more_events: false })"#,
    ),
    ("generic/local", "02 01040205000f0c00000001", r#"Ok(b"")"#),
    (
        "generic/routed",
        "09 012400020105ff0204000f0c00000001",
        r#"Ok(b"")"#,
    ),
    (
        "ges/local",
        "02 010402050004090229005904",
        r#"Ok(GetEnrollmentSummaryAck { entries: [] })"#,
    ),
    (
        "ges/routed",
        "09 012400020105ff02040004090229005904",
        r#"Ok(GetEnrollmentSummaryAck { entries: [] })"#,
    ),
    (
        "log/local",
        "02 01040205001a0c0500000119836e210531026f",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "log/routed",
        "09 012400020105ff0204001a0c0500000119836e210531026f",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "lso-noobject/local",
        "02 01040205001b09041d09006f70657261746f722904",
        r#"Ok(())"#,
    ),
    (
        "lso/local",
        "02 01040205001b09041d09006f70657261746f7229023c05400002",
        r#"Ok(())"#,
    ),
    (
        "lso/routed",
        "09 012400020105ff0204001b09041d09006f70657261746f7229023c05400002",
        r#"Ok(())"#,
    ),
    (
        "reinit-error/local",
        "02 0104020500140901",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "reinit/local",
        "02 01040205001409011d0700736563726574",
        r#"Ok(())"#,
    ),
    (
        "reinit/routed",
        "09 012400020105ff0204001409011d0700736563726574",
        r#"Ok(())"#,
    ),
    (
        "remove-list/local",
        "02 0104020500090c068000011a015929013e21073f",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "remove-list/routed",
        "09 012400020105ff020400090c068000011a015929013e21073f",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "rp-mismatch/local",
        "02 01040205000c0c000000011955",
        r#"Err(Decoding(DecodeError { offset: 0, kind: InvalidEncoding, context: "", label: "ReadProperty ACK object identifier does not match the request", values: [0, 0], index: None }))"#,
    ),
    (
        "rp-mismatch/routed",
        "09 012400020105ff0204000c0c000000011955",
        r#"Err(Decoding(DecodeError { offset: 0, kind: InvalidEncoding, context: "", label: "ReadProperty ACK object identifier does not match the request", values: [0, 0], index: None }))"#,
    ),
    (
        "rp/local",
        "02 01040205000c0c000000011955",
        r#"Ok(ReadPropertyACK { object_identifier: ObjectIdentifier(ObjectType::ANALOG_INPUT, 1), property_identifier: PropertyIdentifier::PRESENT_VALUE, property_array_index: None, property_value: [68, 65, 168, 0, 0] })"#,
    ),
    (
        "rp/routed",
        "09 012400020105ff0204000c0c000000011955",
        r#"Ok(ReadPropertyACK { object_identifier: ObjectIdentifier(ObjectType::ANALOG_INPUT, 1), property_identifier: PropertyIdentifier::PRESENT_VALUE, property_array_index: None, property_value: [68, 65, 168, 0, 0] })"#,
    ),
    (
        "rpm/local",
        "02 01040205000e0c000000011e0955095719031f",
        r#"Ok(ReadPropertyMultipleACK { list_of_read_access_results: [ReadAccessResult { object_identifier: ObjectIdentifier(ObjectType::ANALOG_INPUT, 1), list_of_results: [ReadResultElement { property_identifier: PropertyIdentifier::PRESENT_VALUE, property_array_index: None, property_value: Some([68, 65, 168, 0, 0]), error: None }] }] })"#,
    ),
    (
        "rpm/routed",
        "09 012400020105ff0204000e0c000000011e0955095719031f",
        r#"Ok(ReadPropertyMultipleACK { list_of_read_access_results: [ReadAccessResult { object_identifier: ObjectIdentifier(ObjectType::ANALOG_INPUT, 1), list_of_results: [ReadResultElement { property_identifier: PropertyIdentifier::PRESENT_VALUE, property_array_index: None, property_value: Some([68, 65, 168, 0, 0]), error: None }] }] })"#,
    ),
    (
        "rr-lenient/local",
        "02 01040205001a0c0500000119833e210131023f",
        r#"Ok(ReadRangeReply { ack: ReadRangeAck { object_identifier: ObjectIdentifier(ObjectType::TREND_LOG, 1), property_identifier: PropertyIdentifier::LOG_BUFFER, property_array_index: None, result_flags: (true, true, false), item_count: 3, item_data: [14, 15, 14, 15, 14, 15], first_sequence_number: None }, violations: [ItemCountExceedsRequest] })"#,
    ),
    (
        "rr-lenient/routed",
        "09 012400020105ff0204001a0c0500000119833e210131023f",
        r#"Ok(ReadRangeReply { ack: ReadRangeAck { object_identifier: ObjectIdentifier(ObjectType::TREND_LOG, 1), property_identifier: PropertyIdentifier::LOG_BUFFER, property_array_index: None, result_flags: (true, true, false), item_count: 3, item_data: [14, 15, 14, 15, 14, 15], first_sequence_number: None }, violations: [ItemCountExceedsRequest] })"#,
    ),
    (
        "rr-strict/local",
        "02 01040205001a0c0500000119833e210131023f",
        r#"Err(ReadRangeViolation(ItemCountExceedsRequest))"#,
    ),
    (
        "rr-strict/routed",
        "09 012400020105ff0204001a0c0500000119833e210131023f",
        r#"Err(ReadRangeViolation(ItemCountExceedsRequest))"#,
    ),
    (
        "rr/local",
        "02 01040205001a0c0500000119833e210131023f",
        r#"Ok(ReadRangeAck { object_identifier: ObjectIdentifier(ObjectType::TREND_LOG, 1), property_identifier: PropertyIdentifier::LOG_BUFFER, property_array_index: None, result_flags: (true, true, false), item_count: 2, item_data: [14, 15, 14, 15], first_sequence_number: None })"#,
    ),
    (
        "rr/routed",
        "09 012400020105ff0204001a0c0500000119833e210131023f",
        r#"Ok(ReadRangeAck { object_identifier: ObjectIdentifier(ObjectType::TREND_LOG, 1), property_identifier: PropertyIdentifier::LOG_BUFFER, property_array_index: None, result_flags: (true, true, false), item_count: 2, item_data: [14, 15, 14, 15], first_sequence_number: None })"#,
    ),
    (
        "scpm-cancel/local",
        "02 01040205001e090c19004e4f",
        r#"Ok(())"#,
    ),
    (
        "scpm/local",
        "02 01040205001e090c19002a025839054e0c000000011e0e09550f1c3f00000029011f4f",
        r#"Ok(())"#,
    ),
    (
        "scpm/routed",
        "09 012400020105ff0204001e090c19002a025839054e0c000000011e0e09550f1c3f00000029011f4f",
        r#"Ok(())"#,
    ),
    ("ts/local", "02 01001006a47e0b0f07b40c1e0500", r#"Ok(())"#),
    (
        "ts/routed",
        "09 012000020105ff1006a47e0b0f07b40c1e0500",
        r#"Ok(())"#,
    ),
    (
        "uaudit-note/local",
        "02 0100100c0e2e0c020000012f4901ae0c02000002af0f",
        r#"Ok(())"#,
    ),
    (
        "uaudit-note/routed",
        "09 012000020105ff100c0e2e0c020000012f4901ae0c02000002af0f",
        r#"Ok(())"#,
    ),
    (
        "ugeneric/local",
        "02 01001009a47e0b0f07b40c000000",
        r#"Ok(())"#,
    ),
    (
        "ugeneric/routed",
        "09 012000020105ff1009a47e0b0f07b40c000000",
        r#"Ok(())"#,
    ),
    ("upt/local", "02 010010040a022b19072e21052f", r#"Ok(())"#),
    (
        "upt/routed",
        "09 012000020105ff10040a022b19072e21052f",
        r#"Ok(())"#,
    ),
    (
        "utm/local",
        "02 010010050c020000091e09031f29013d0b00636865636b2070756d70",
        r#"Ok(())"#,
    ),
    (
        "utm/routed",
        "09 012000020105ff10050c020000091e09031f29013d0b00636865636b2070756d70",
        r#"Ok(())"#,
    ),
    ("uts/local", "02 01001009a47e0b0f07b40c1e0500", r#"Ok(())"#),
    (
        "uts/routed",
        "09 012000020105ff1009a47e0b0f07b40c1e0500",
        r#"Ok(())"#,
    ),
    ("vt-close/local", "02 010402050016211d211e", r#"Ok(())"#),
    (
        "vt-close/routed",
        "09 012400020105ff02040016211d211e",
        r#"Ok(())"#,
    ),
    (
        "vt-data/local",
        "02 010402050017211d636c730d2101",
        r#"Ok(Partial { accepted_octet_count: 2 })"#,
    ),
    (
        "vt-data/routed",
        "09 012400020105ff02040017211d636c730d2101",
        r#"Ok(Partial { accepted_octet_count: 2 })"#,
    ),
    (
        "vt-open/local",
        "02 01040205001591032105",
        r#"Ok(VTOpenAck { remote_vt_session_identifier: 29 })"#,
    ),
    (
        "vt-open/routed",
        "09 012400020105ff0204001591032105",
        r#"Ok(VTOpenAck { remote_vt_session_identifier: 29 })"#,
    ),
    (
        "wg/device",
        "02 0100100a091719082e090c4442860000090d190a002f3900",
        r#"Ok(())"#,
    ),
    (
        "wg/global",
        " 0120ffff00ff100a091719082e090c4442860000090d190a002f3900",
        r#"Ok(())"#,
    ),
    (
        "wg/local",
        " 0100100a091719082e090c4442860000090d190a002f3900",
        r#"Ok(())"#,
    ),
    (
        "wg/network",
        " 0120000200ff100a091719082e090c4442860000090d190a002f3900",
        r#"Ok(())"#,
    ),
    (
        "wg/own",
        " 0100100a091719082e090c4442860000090d190a002f3900",
        r#"Ok(())"#,
    ),
    (
        "wg/routed",
        "09 012000020105ff100a091719082e090c4442860000090d190a002f3900",
        r#"Ok(())"#,
    ),
    (
        "who-am-i-bcast/global",
        " 0120ffff00ff100d22022b7505004d582d3175050030303432",
        r#"Ok(())"#,
    ),
    (
        "who-am-i-bcast/local",
        " 0100100d22022b7505004d582d3175050030303432",
        r#"Ok(())"#,
    ),
    (
        "who-am-i-bcast/network",
        " 0120000200ff100d22022b7505004d582d3175050030303432",
        r#"Ok(())"#,
    ),
    (
        "who-am-i-bcast/own",
        " 0100100d22022b7505004d582d3175050030303432",
        r#"Ok(())"#,
    ),
    (
        "who-am-i/local",
        "02 0100100d22022b7505004d582d3175050030303432",
        r#"Ok(())"#,
    ),
    (
        "who-am-i/routed",
        "09 012000020105ff100d22022b7505004d582d3175050030303432",
        r#"Ok(())"#,
    ),
    (
        "who-has-bcast/global",
        " 0120ffff00ff1007090a19143d050070756d70",
        r#"Ok(())"#,
    ),
    (
        "who-has-bcast/global-id",
        " 0120ffff00ff10072c00000001",
        r#"Ok(())"#,
    ),
    (
        "who-has-bcast/local",
        " 01001007090a19143d050070756d70",
        r#"Ok(())"#,
    ),
    (
        "who-has-bcast/network",
        " 0120000200ff1007090a19143d050070756d70",
        r#"Ok(())"#,
    ),
    (
        "who-has-bcast/own",
        " 01001007090a19143d050070756d70",
        r#"Ok(())"#,
    ),
    (
        "who-has/local",
        "02 01001007090a19143d050070756d70",
        r#"Ok(())"#,
    ),
    (
        "who-has/routed",
        "09 012000020105ff1007090a19143d050070756d70",
        r#"Ok(())"#,
    ),
    (
        "who-is-bcast/global",
        " 0120ffff00ff1008090a1914",
        r#"Ok(())"#,
    ),
    ("who-is-bcast/global-all", " 0120ffff00ff1008", r#"Ok(())"#),
    ("who-is-bcast/local", " 01001008090a1914", r#"Ok(())"#),
    (
        "who-is-bcast/network",
        " 0120000200ff1008090a1914",
        r#"Ok(())"#,
    ),
    ("who-is-bcast/own", " 01001008090a1914", r#"Ok(())"#),
    ("who-is/local", "02 01001008", r#"Ok(())"#),
    ("who-is/routed", "09 012000020105ff1008", r#"Ok(())"#),
    (
        "wp-error/local",
        "02 01040205000f0c00400001195529023e44429100003f",
        r#"Err(Remote(Error { class: ErrorClass::OBJECT, code: ErrorCode::UNKNOWN_OBJECT }))"#,
    ),
    (
        "wp/local",
        "02 01040205000f0c0040000119553e44429100003f4908",
        r#"Ok(())"#,
    ),
    (
        "wp/routed",
        "09 012400020105ff0204000f0c0040000119553e44429100003f4908",
        r#"Ok(())"#,
    ),
    (
        "wpm/local",
        "02 0104020500100c008000031e09552e44429100002f390a1f",
        r#"Ok(())"#,
    ),
    (
        "wpm/routed",
        "09 012400020105ff020400100c008000031e09552e44429100002f390a1f",
        r#"Ok(())"#,
    ),
];
