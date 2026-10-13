//! Audit notifications under DeviceCommunicationControl (#1370).
//!
//! Clause 16.1 keeps Confirmed- and UnconfirmedAuditNotification going while
//! DISABLE_INITIATION is in force, so every target audit path sends as usual
//! and its delivery alone decides the Reporter's health. COV and event
//! notifications stay held back under the same state.
use super::*;
use crate::server::event_notifications_tests::local_broadcast_destination;
use bacnet_objects::{
    analog::AnalogValueObject,
    audit::{AuditReporterObject, ObjectAuditPolicy},
    notification_class::NotificationClass,
    traits::BACnetObject,
};
use bacnet_services::{cov::SubscribeCOVRequest, device_mgmt::DeviceCommunicationControlRequest};
use bacnet_types::{constructed::BACnetDestination, enums::EnableDisable};

/// Put the server under DISABLE_INITIATION through a wire request, as a peer
/// would, so the one-minute DCC timer is live and re-enables on its own.
async fn disable_initiation(f: &mut Fixture) {
    f.server.config_mut().dcc_policy = DccPolicy::LegacyPermissive;
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: Some(1),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            data.freeze()
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);
}

/// Let the DCC timer run out and check that initiation is enabled again.
async fn expire_dcc(f: &Fixture) {
    tokio::time::advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(
        f.server.comm_state(),
        DccState::Enable,
        "the DCC timer re-enables"
    );
}

/// Every audit notification on the wire, with the invoke ID of a confirmed one.
fn audit_frames(f: &Fixture) -> Vec<(Option<u8>, BACnetAuditNotification)> {
    use bacnet_services::audit::AuditNotificationRequest;
    let mut frames = Vec::new();
    for bytes in f.transport.sent.lock().unwrap().iter() {
        let (invoke, service) =
            match decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap() {
                Apdu::ConfirmedRequest(request) => {
                    assert_eq!(
                        request.service_choice,
                        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
                    );
                    (Some(request.invoke_id), request.service_request)
                }
                Apdu::UnconfirmedRequest(request) => {
                    assert_eq!(
                        request.service_choice,
                        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
                    );
                    (None, request.service_request)
                }
                other => panic!("unexpected {other:?}"),
            };
        for notification in AuditNotificationRequest::decode(&service)
            .unwrap()
            .notifications
        {
            frames.push((invoke, notification));
        }
    }
    frames
}

fn ack(f: &Fixture, invoke: u8) {
    assert!(f.server.notification_transactions.admit_terminal(
        LOGGER,
        None,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: invoke,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        })
    ));
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

#[tokio::test(start_paused = true)]
async fn audit_notifications_start_under_disable_initiation_and_keep_the_reporter_healthy() {
    for (confirmed, delayed) in [(false, false), (true, false), (false, true), (true, true)] {
        let case = format!("confirmed={confirmed} delayed={delayed}");
        let mut reporter = if delayed {
            batching::delayed(1)
        } else {
            reporter()
        };
        reporter
            .set_issue_confirmed_notifications(confirmed)
            .unwrap();
        let mut f = server(reporter).await;
        disable_initiation(&mut f).await;
        assert!(matches!(
            write_value(&f.server, None).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        if delayed {
            assert!(audit_frames(&f).is_empty(), "{case}: waits for its delay");
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
        }
        assert_eq!(f.server.comm_state(), DccState::DisableInitiation, "{case}");
        let frames = audit_frames(&f);
        assert_eq!(frames.len(), 1, "{case}: sent under DISABLE_INITIATION");
        assert_eq!(frames[0].0.is_some(), confirmed, "{case}");
        assert_eq!(
            frames[0].1.target_object,
            Some(oid(ObjectType::BINARY_VALUE, 1))
        );
        if let Some(invoke) = frames[0].0 {
            ack(&f, invoke);
            settle().await;
        }
        assert_eq!(
            health(&f.server).await,
            Reliability::NO_FAULT_DETECTED,
            "{case}"
        );
        // Re-enabling resends nothing: the record went out once, under DCC.
        expire_dcc(&f).await;
        assert_eq!(audit_frames(&f).len(), 1, "{case}: not sent again");
        assert_eq!(
            health(&f.server).await,
            Reliability::NO_FAULT_DETECTED,
            "{case}"
        );
        f.server.stop().await.unwrap();
    }
}

/// DCC taking effect while a confirmed audit notification waits for its
/// answer leaves it alone: the acknowledgment still delivers it.
#[tokio::test(start_paused = true)]
async fn an_outstanding_audit_notification_survives_disable_initiation_to_its_answer() {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut f = server(reporter).await;
    assert!(matches!(
        write_value(&f.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1);
    disable_initiation(&mut f).await;
    tokio::time::advance(Duration::from_millis(1500)).await;
    settle().await;
    assert_eq!(
        f.server.notification_transactions.active_count(),
        1,
        "still waiting for its answer"
    );
    ack(&f, frames[0].0.expect("confirmed"));
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(audit_frames(&f).len(), 1, "one attempt, no retry");
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}

/// COV and event notifications seen at `SOURCE`, which subscribes and is the
/// Notification Class recipient: (COV, event) counts.
fn cov_and_events(f: &Fixture) -> (usize, usize) {
    let (mut cov, mut events) = (0, 0);
    for bytes in f.transport.responses.lock().unwrap().iter() {
        let service = match decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap() {
            Apdu::UnconfirmedRequest(request) => request.service_choice,
            _ => continue,
        };
        if service == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION {
            cov += 1;
        } else if service == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION {
            events += 1;
        }
    }
    (cov, events)
}

async fn write_watched(f: &Fixture, value: f32) {
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::ANALOG_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                encoded(&PropertyValue::Real(value)),
                None,
            ),
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
}

/// The exemption covers audit only. One write under DISABLE_INITIATION that
/// changes a subscribed value and crosses its high limit sends its audit
/// record but neither the COV nor the event notification, and re-enabling
/// sends none of them again.
#[tokio::test(start_paused = true)]
async fn disable_initiation_still_holds_back_cov_and_events_beside_audit() {
    let mut f = server(reporter()).await;
    {
        let mut db = f.server.db.write().await;
        let mut class = NotificationClass::new(0, "NC-0").unwrap();
        class
            .add_destination(BACnetDestination {
                recipient: BACnetRecipient::Address(BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(SOURCE),
                }),
                ..local_broadcast_destination()
            })
            .unwrap();
        db.add(Box::new(class)).unwrap();
        let mut watched = AnalogValueObject::new(1, "watched", 62).unwrap();
        for (property, value) in [
            (PropertyIdentifier::HIGH_LIMIT, PropertyValue::Real(80.0)),
            (PropertyIdentifier::LOW_LIMIT, PropertyValue::Real(20.0)),
            (PropertyIdentifier::DEADBAND, PropertyValue::Real(2.0)),
            (
                PropertyIdentifier::LIMIT_ENABLE,
                PropertyValue::BitString {
                    unused_bits: 6,
                    data: vec![0xC0],
                },
            ),
            (
                PropertyIdentifier::EVENT_ENABLE,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0xE0],
                },
            ),
        ] {
            watched.write_property(property, None, value, None).unwrap();
        }
        db.add(Box::new(watched)).unwrap();
    }
    let mut subscribe = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid(ObjectType::ANALOG_VALUE, 1),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(0),
    }
    .encode(&mut subscribe)
    .unwrap();
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::SUBSCRIBE_COV,
            subscribe.freeze()
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(
        cov_and_events(&f),
        (1, 0),
        "the subscription's first report"
    );

    disable_initiation(&mut f).await;
    write_watched(&f, 90.0).await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1, "the write's audit record goes out");
    assert_eq!(
        frames[0].1.target_object,
        Some(oid(ObjectType::ANALOG_VALUE, 1))
    );
    assert_eq!(cov_and_events(&f), (1, 0), "COV and event stay held back");

    expire_dcc(&f).await;
    assert_eq!(audit_frames(&f).len(), 1, "nothing is resent");
    assert_eq!(cov_and_events(&f), (1, 0), "nothing is resent");

    // Enabled again, the same setup reports a change both ways once.
    write_watched(&f, 50.0).await;
    assert_eq!(audit_frames(&f).len(), 2);
    assert_eq!(cov_and_events(&f), (2, 1));
    f.server.stop().await.unwrap();
}

/// Writes whose commit owes an audit notification went to
/// SERVICE_REQUEST_DENIED while initiation was disabled, because the
/// notification could not be sent. They commit and report like any other.
#[tokio::test(start_paused = true)]
async fn audit_owing_writes_commit_and_notify_under_disable_initiation() {
    let recipient = |instance| BACnetRecipient::Device(oid(ObjectType::DEVICE, instance));
    for case in ["recipient", "reporter", "send now", "object policy"] {
        let reporter: AuditReporterObject = if case == "send now" {
            batching::delayed(30)
        } else {
            reporter()
        };
        let mut f = try_server(
            reporter,
            &[10],
            Some(recipient(20)),
            vec![
                DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
                DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap(),
            ],
        )
        .await
        .unwrap();
        if case == "object policy" {
            let mut value = AnalogValueObject::new(11, "policy", 62).unwrap();
            let mut operations = AuditOperationFlags::empty();
            operations.insert(AuditOperation::WRITE);
            value.set_audit_policy(ObjectAuditPolicy {
                level: Some(AuditLevel::AUDIT_ALL),
                operations: Some(operations),
                ..Default::default()
            });
            f.server.db.write().await.add(Box::new(value)).unwrap();
        }
        disable_initiation(&mut f).await;
        let reporter_oid = oid(ObjectType::AUDIT_REPORTER, 1);
        let (target, property, value) = match case {
            "recipient" => {
                let mut value = BytesMut::new();
                bacnet_encoding::constructed::encode_recipient(&mut value, &recipient(21)).unwrap();
                (
                    oid(ObjectType::DEVICE, 10),
                    PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    value.to_vec(),
                )
            }
            "reporter" => (
                reporter_oid,
                PropertyIdentifier::DESCRIPTION,
                encoded(&PropertyValue::CharacterString("audited".into())),
            ),
            "send now" => (
                reporter_oid,
                PropertyIdentifier::SEND_NOW,
                encoded(&PropertyValue::Boolean(true)),
            ),
            _ => (
                oid(ObjectType::ANALOG_VALUE, 11),
                PropertyIdentifier::AUDIT_LEVEL,
                encoded(&PropertyValue::Enumerated(
                    AuditLevel::AUDIT_CONFIG.to_raw(),
                )),
            ),
        };
        let response = dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(target, property, value, None),
        )
        .await;
        assert!(
            matches!(response, Apdu::SimpleAck(_)),
            "{case}: {response:?}"
        );
        settle().await;
        let frames = audit_frames(&f);
        // A recipient change reports to the old and the new recipient.
        let expected = if case == "recipient" { 2 } else { 1 };
        assert_eq!(frames.len(), expected, "{case}");
        for (_, notification) in &frames {
            assert_eq!(notification.target_object, Some(target), "{case}");
            assert_eq!(
                notification
                    .target_property
                    .as_ref()
                    .map(|reference| reference.property_identifier),
                Some(property),
                "{case}"
            );
        }
        if case == "recipient" {
            assert_eq!(
                *f.transport.destinations.lock().unwrap(),
                vec![LOGGER.to_vec(), NEW_LOGGER.to_vec()]
            );
        }
        assert_eq!(
            health(&f.server).await,
            Reliability::NO_FAULT_DETECTED,
            "{case}"
        );
        f.server.stop().await.unwrap();
    }
}

/// Send_Now under DISABLE_INITIATION flushes a delayed Reporter's queue: the
/// record queued before DCC and the one queued under it go out with the
/// command's own record, and neither the send delay running out nor the DCC
/// timer re-enabling initiation sends them again.
#[tokio::test(start_paused = true)]
async fn send_now_under_disable_initiation_flushes_the_queue_once() {
    let mut f = server(batching::delayed(30)).await;
    assert!(matches!(
        write_value(&f.server, None).await,
        Apdu::SimpleAck(_)
    ));
    disable_initiation(&mut f).await;
    assert!(matches!(
        write_value(&f.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert!(audit_frames(&f).is_empty(), "both wait for the send delay");
    batching::command(&f, true).await;
    settle().await;
    let count = |f: &Fixture| {
        let frames = audit_frames(f);
        let queued = frames
            .iter()
            .filter(|(_, n)| n.target_object == Some(oid(ObjectType::BINARY_VALUE, 1)))
            .count();
        let commands = frames
            .iter()
            .filter(|(_, n)| {
                n.target_property.as_ref().map(|p| p.property_identifier)
                    == Some(PropertyIdentifier::SEND_NOW)
            })
            .count();
        (queued, commands, frames.len())
    };
    assert_eq!(
        count(&f),
        (2, 1, 3),
        "queue flushed with the command record"
    );
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    // Sixty seconds also runs past the 30-second send delay.
    expire_dcc(&f).await;
    assert_eq!(count(&f), (2, 1, 3), "nothing is resent");
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}

/// A record dropped for want of a send slot under DISABLE_INITIATION is a
/// resource loss like any other, and its AUDITING_FAILURE summary goes out.
#[tokio::test(start_paused = true)]
async fn a_resource_drop_under_disable_initiation_is_summarized() {
    let mut reporter = reporter();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    operations.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(operations).unwrap();
    let mut f = server(reporter).await;
    disable_initiation(&mut f).await;
    let permits: Vec<_> = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    for _ in 0..2 {
        assert!(matches!(
            write_value(&f.server, None).await,
            Apdu::SimpleAck(_)
        ));
    }
    settle().await;
    assert!(audit_frames(&f).is_empty());
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (true, 2, 0),
        "both records count as dropped"
    );
    drop(permits);
    settle().await;
    let frames = audit_frames(&f);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1.operation, AuditOperation::AUDITING_FAILURE);
    assert_eq!(frames[0].1.current_value, Some(vec![0x21, 2]));
    assert_eq!(f.server.comm_state(), DccState::DisableInitiation);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    f.server.stop().await.unwrap();
}

#[path = "audit_dcc_record_tests.rs"]
mod records;
