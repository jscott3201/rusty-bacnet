//! [`SENT_SERVICES`] matches what the client's methods put on the wire.
//!
//! One test calls every sending method once against a scripted peer that
//! records each request and answers every confirmed one with an Error, then
//! compares the services it saw with the list. The other reads the client's
//! source and fails on a service choice the list leaves out, so a new sending
//! method can't skip the list without failing here.
use super::*;

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::sync::Mutex as StdMutex;

use bacnet_encoding::apdu::{ErrorPdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::audit::{
    AuditLogQueryRequest, AuditNotificationRequest, BACnetAuditLogQueryParameters,
    BACnetAuditNotification,
};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::enrollment_summary::GetEnrollmentSummaryRequest;
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::constructed::{BACnetRecipient, PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    AcknowledgmentFilter, AuditOperation, BACnetSuccessFilter, EnableDisable, ErrorClass,
    ErrorCode, EventState, ObjectType, PropertyIdentifier, ReinitializedState, ServiceSupported,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};

use super::cov::CovPropertySubscription;
use super::write_group::WriteGroupDestination;

const CLIENT_MAC: [u8; 1] = [0x01];
const PEER: [u8; 1] = [0x02];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A started client and the services its peer has received so far. The peer
/// answers each confirmed request with an Error, so every method returns.
async fn client_and_log() -> (
    BACnetClient<LoopbackTransport>,
    Arc<StdMutex<BTreeSet<u8>>>,
    JoinHandle<()>,
) {
    let (client_transport, mut peer) = LoopbackTransport::pair(CLIENT_MAC.to_vec(), PEER.to_vec());
    let mut inbound = peer.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let seen = Arc::new(StdMutex::new(BTreeSet::new()));
    let log = Arc::clone(&seen);
    let task = tokio::spawn(async move {
        while let Some(received) = inbound.recv().await {
            let Ok(npdu) = decode_npdu(received.npdu) else {
                continue;
            };
            let request = match decode_apdu(npdu.payload) {
                Ok(Apdu::ConfirmedRequest(request)) => request,
                Ok(Apdu::UnconfirmedRequest(request)) => {
                    let bit = ServiceSupported::from_unconfirmed_choice(request.service_choice)
                        .expect("the client sends only defined services");
                    log.lock().unwrap().insert(bit.to_raw());
                    continue;
                }
                _ => continue,
            };
            let bit = ServiceSupported::from_confirmed_choice(request.service_choice)
                .expect("the client sends only defined services");
            log.lock().unwrap().insert(bit.to_raw());
            let error = Apdu::Error(ErrorPdu {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::SERVICE_REQUEST_DENIED,
                error_data: Bytes::new(),
            });
            let mut apdu_buf = BytesMut::new();
            encode_apdu(&mut apdu_buf, &error).unwrap();
            let mut npdu_buf = BytesMut::new();
            encode_npdu(
                &mut npdu_buf,
                &Npdu {
                    payload: apdu_buf.freeze(),
                    ..Npdu::default()
                },
            )
            .unwrap();
            if peer.send_unicast(&npdu_buf, &CLIENT_MAC).await.is_err() {
                break;
            }
        }
    });
    (client, seen, task)
}

fn audit_notification() -> AuditNotificationRequest {
    AuditNotificationRequest {
        notifications: vec![BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: None,
            source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
            source_object: None,
            operation: AuditOperation::WRITE,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        }],
    }
}

/// Call every method that sends a service once. Each confirmed one fails on
/// the peer's Error; only what reached the wire matters here. The last call
/// is a confirmed request, so when this returns the peer has logged them all.
async fn send_everything(client: &BACnetClient<LoopbackTransport>) {
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let file = oid(ObjectType::FILE, 1);
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let date = Date {
        year: 126,
        month: 10,
        day: 25,
        day_of_week: 7,
    };
    let time = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    // Unconfirmed first: the peer handles requests in order, so once the last
    // confirmed request's Error is back, every request before it is logged.
    client
        .time_synchronization(&PEER, date, time)
        .await
        .unwrap();
    client
        .utc_time_synchronization(&PEER, date, time)
        .await
        .unwrap();
    client.who_is(None).await.unwrap();
    client
        .who_has(WhoHasObject::Identifier(ai), None)
        .await
        .unwrap();
    client
        .write_group(
            &WriteGroupDestination::Device(MacAddr::from_slice(&PEER)),
            &WriteGroupRequest {
                group_number: NonZeroU32::new(1).unwrap(),
                write_priority: 8,
                change_list: vec![GroupChannelValue {
                    channel: 1,
                    override_priority: None,
                    value: vec![0x00],
                }],
                inhibit_delay: None,
            },
        )
        .await
        .unwrap();
    let _ = client
        .read_property_multiple(
            &PEER,
            vec![ReadAccessSpecification {
                object_identifier: ai,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: pv,
                    property_array_index: None,
                }],
            }],
        )
        .await;
    let _ = client
        .write_property(&PEER, ai, pv, None, vec![0x00], None)
        .await;
    let _ = client
        .write_property_multiple(
            &PEER,
            vec![WriteAccessSpecification {
                object_identifier: ai,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: pv,
                    property_array_index: None,
                    value: vec![0x00],
                    priority: None,
                }],
            }],
        )
        .await;
    let _ = client.read_range(&PEER, ai, pv, None, None).await;
    let _ = client.subscribe_cov(&PEER, 1, ai, false, Some(60)).await;
    let _ = client
        .subscribe_cov_property(
            &PEER,
            CovPropertySubscription {
                subscriber_process_identifier: 1,
                monitored_object_identifier: ai,
                monitored_property_identifier: pv,
                monitored_property_array_index: None,
                confirmed: false,
                lifetime: NonZeroU32::new(60).unwrap(),
                cov_increment: None,
            },
        )
        .await;
    let _ = client
        .acknowledge_alarm_request(
            &PEER,
            &AcknowledgeAlarmRequest {
                acknowledging_process_identifier: 1,
                event_object_identifier: ai,
                event_state_acknowledged: EventState::OFFNORMAL,
                timestamp: BACnetTimeStamp::SequenceNumber(1),
                acknowledgment_source: "operator".into(),
                time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(2),
            },
        )
        .await;
    let _ = client.get_alarm_summary(&PEER).await;
    let _ = client
        .get_enrollment_summary(
            &PEER,
            &GetEnrollmentSummaryRequest {
                acknowledgment_filter: AcknowledgmentFilter::ALL,
                enrollment_filter: None,
                event_state_filter: None,
                event_type_filter: None,
                priority_filter: None,
                notification_class_filter: None,
            },
        )
        .await;
    let _ = client.get_event_information(&PEER, None).await;
    let _ = client
        .atomic_read_file(
            &PEER,
            file,
            FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 16,
            },
        )
        .await;
    let _ = client
        .atomic_write_file(
            &PEER,
            file,
            FileWriteAccessMethod::Stream {
                file_start_position: 0,
                file_data: vec![1],
            },
        )
        .await;
    let list = PropertyIdentifier::MEMBER_OF;
    let _ = client
        .add_list_element(&PEER, ai, list, None, vec![0x00])
        .await;
    let _ = client
        .remove_list_element(&PEER, ai, list, None, vec![0x00])
        .await;
    let _ = client
        .create_object(
            &PEER,
            ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            Vec::new(),
        )
        .await;
    let _ = client.delete_object(&PEER, ai).await;
    let _ = client
        .device_communication_control(&PEER, EnableDisable::ENABLE, None, None)
        .await;
    let _ = client
        .reinitialize_device(&PEER, ReinitializedState::WARMSTART, None)
        .await;
    let _ = client
        .confirmed_audit_notification(&PEER, &audit_notification())
        .await;
    let _ = client
        .unconfirmed_audit_notification(&PEER, &audit_notification())
        .await;
    let _ = client
        .audit_log_query(
            &PEER,
            &AuditLogQueryRequest {
                audit_log: oid(ObjectType::AUDIT_LOG, 1),
                query_parameters: BACnetAuditLogQueryParameters::BySource {
                    source_device_identifier: oid(ObjectType::DEVICE, 1),
                    source_device_address: None,
                    source_object_identifier: None,
                    operations: None,
                    successful_actions_only: BACnetSuccessFilter::ALL,
                },
                start_at_sequence_number: None,
                requested_count: 1,
            },
        )
        .await;
    let _ = client.read_property(&PEER, ai, pv, None).await;
}

#[tokio::test]
async fn the_methods_send_exactly_the_listed_services() {
    let (client, seen, task) = client_and_log().await;
    send_everything(&client).await;
    let expected: BTreeSet<u8> = SENT_SERVICES.iter().map(|s| s.to_raw()).collect();
    assert_eq!(
        expected.len(),
        SENT_SERVICES.len(),
        "no service listed twice"
    );
    let seen = seen.lock().unwrap().clone();
    let name = |raw: &u8| ServiceSupported::from_raw(*raw).to_string();
    let unlisted: Vec<_> = seen.difference(&expected).map(name).collect();
    let unsent: Vec<_> = expected.difference(&seen).map(name).collect();
    assert!(
        unlisted.is_empty() && unsent.is_empty(),
        "SENT_SERVICES must match the wire: sent but not listed {unlisted:?}, listed but not sent {unsent:?}"
    );
    task.abort();
}

/// Every non-test source file of the crate, except the inbound dispatcher,
/// which names the services the client receives.
fn sending_sources() -> Vec<(std::path::PathBuf, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if !name.contains("test") {
                    walk(&path, out);
                }
            } else if name.ends_with(".rs")
                && !name.contains("test")
                && !matches!(name.as_str(), "dispatch.rs" | "fake_device.rs")
            {
                out.push((path.clone(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    out
}

#[test]
fn every_service_choice_in_the_client_source_is_listed() {
    let listed: BTreeSet<u8> = SENT_SERVICES.iter().map(|s| s.to_raw()).collect();
    let sources = sending_sources();
    assert!(
        sources
            .iter()
            .any(|(path, _)| path.ends_with("client/property.rs")),
        "the scan must reach the client's method files"
    );
    let mut missing = Vec::new();
    let mut named = BTreeSet::new();
    for (path, source) in &sources {
        // Inline test modules may name anything.
        let source = strip_test_modules(source);
        let source = source.as_str();
        for (prefix, confirmed) in [
            ("UnconfirmedServiceChoice::", false),
            ("ConfirmedServiceChoice::", true),
        ] {
            for (index, _) in source.match_indices(prefix) {
                // "UnconfirmedServiceChoice::" contains "ConfirmedServiceChoice::".
                if confirmed && source[..index].ends_with("Un") {
                    continue;
                }
                let rest = &source[index + prefix.len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                    .collect();
                if name.is_empty() {
                    continue;
                }
                let bit = if confirmed {
                    name.parse::<ConfirmedServiceChoice>()
                        .ok()
                        .and_then(ServiceSupported::from_confirmed_choice)
                } else {
                    name.parse::<UnconfirmedServiceChoice>()
                        .ok()
                        .and_then(ServiceSupported::from_unconfirmed_choice)
                };
                let bit = bit.unwrap_or_else(|| panic!("{name} in {path:?} is a defined service"));
                named.insert(bit.to_raw());
                if !listed.contains(&bit.to_raw()) {
                    missing.push(format!("{name} ({})", path.display()));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "the client source sends services SENT_SERVICES leaves out: {missing:?}"
    );
    // The scan saw the whole source: each listed service is named in it.
    let unseen: Vec<_> = listed.difference(&named).collect();
    assert!(
        unseen.is_empty(),
        "listed services the scan didn't see: {unseen:?}"
    );
}

/// Drops each `#[cfg(test)] mod name { .. }` block, so inline tests don't
/// count. Everything else stays, including `#[cfg(test)]` on single items
/// such as a `use` (#1496 review: cutting at the first marker skipped most
/// of `client/mod.rs`). A copy lives in `bacnet-server`'s
/// `pics/initiated_tests.rs`; keep the two alike.
fn strip_test_modules(source: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find(MARKER) {
        let after = &rest[at + MARKER.len()..];
        let trimmed = after.trim_start();
        let is_inline_module = trimmed.starts_with("mod ")
            && trimmed
                .find(['{', ';'])
                .is_some_and(|i| trimmed.as_bytes()[i] == b'{');
        if !is_inline_module {
            out.push_str(&rest[..at + MARKER.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..at]);
        let open = after.find('{').expect("checked above");
        let mut depth = 0usize;
        let mut end = after.len();
        for (index, ch) in after[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + index + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn stripping_drops_only_inline_test_modules() {
    let source = "#[cfg(test)]\nuse a;\nfn kept() {}\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod inline {\n    fn f() { {} }\n}\nfn also_kept() {}\n";
    assert_eq!(
        strip_test_modules(source),
        "#[cfg(test)]\nuse a;\nfn kept() {}\n#[cfg(test)]\nmod tests;\n\nfn also_kept() {}\n"
    );
}
