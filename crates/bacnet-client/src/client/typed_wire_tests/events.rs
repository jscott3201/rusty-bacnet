//! Event and alarm summaries, COV subscriptions and the Audit services.

use std::num::NonZeroU32;

use bacnet_services::alarm_event::GetEventInformationAck;
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::audit::{
    AuditLogQueryRequest, AuditNotificationRequest, BACnetAuditLogQueryParameters,
    BACnetAuditNotification,
};
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{
    AcknowledgmentFilter, AuditOperation, BACnetSuccessFilter, EnrollmentSummaryEventStateFilter,
    ObjectType,
};

use super::*;
use crate::client::cov::CovPropertySubscription;

#[tokio::test]
async fn get_event_information() {
    let ack = Answer::Complex(octets(|buf| {
        GetEventInformationAck {
            list_of_event_summaries: Vec::new(),
            more_events: false,
        }
        .encode(buf)
        .unwrap()
    }));
    let after = Some(oid(ObjectType::BINARY_INPUT, 3));
    pin!("gei/local", ack.clone(), |c| c
        .get_event_information(PEER, after));
    pin!("gei/local", ack.clone(), |c| c.get_event_information(
        &c.resolve_device(LOCAL).await.unwrap(),
        after
    ));
    pin!("gei/routed", ack, |c| c.get_event_information(
        &c.resolve_device(REMOTE).await.unwrap(),
        after
    ));
}

#[tokio::test]
async fn get_alarm_summary() {
    let ack = Answer::Complex(octets(|buf| {
        GetAlarmSummaryAck {
            entries: Vec::new(),
        }
        .encode(buf)
    }));
    pin!("gas/local", ack.clone(), |c| c.get_alarm_summary(PEER));
    pin!("gas/local", ack.clone(), |c| c
        .get_alarm_summary(&c.resolve_device(LOCAL).await.unwrap()));
    pin!("gas/routed", ack, |c| c
        .get_alarm_summary(&c.resolve_device(REMOTE).await.unwrap()));
}

#[tokio::test]
async fn get_enrollment_summary() {
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: AcknowledgmentFilter::NOT_ACKED,
        enrollment_filter: None,
        event_state_filter: Some(EnrollmentSummaryEventStateFilter::OFFNORMAL),
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: Some(4),
    };
    let ack = Answer::Complex(octets(|buf| {
        GetEnrollmentSummaryAck {
            entries: Vec::new(),
        }
        .encode(buf)
    }));
    pin!("ges/local", ack.clone(), |c| c
        .get_enrollment_summary(PEER, &request));
    pin!("ges/local", ack.clone(), |c| c.get_enrollment_summary(
        &c.resolve_device(LOCAL).await.unwrap(),
        &request
    ));
    pin!("ges/routed", ack, |c| c.get_enrollment_summary(
        &c.resolve_device(REMOTE).await.unwrap(),
        &request
    ));
}

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

#[tokio::test]
async fn subscribe_cov() {
    pin!("cov/local", Answer::Simple, |c| c.subscribe_cov(
        PEER,
        9,
        ai1(),
        true,
        Some(300)
    ));
    pin!("cov/local", Answer::Simple, |c| c.subscribe_cov(
        &c.resolve_device(LOCAL).await.unwrap(),
        9,
        ai1(),
        true,
        Some(300)
    ));
    pin!("cov/routed", Answer::Simple, |c| c.subscribe_cov(
        &c.resolve_device(REMOTE).await.unwrap(),
        9,
        ai1(),
        true,
        Some(300)
    ));
    pin!("cov-cancel/local", Answer::Simple, |c| c.unsubscribe_cov(
        PEER,
        9,
        ai1()
    ));
    pin!("cov-cancel/local", Answer::Simple, |c| c.unsubscribe_cov(
        &c.resolve_device(LOCAL).await.unwrap(),
        9,
        ai1()
    ));
    pin!("cov-cancel/routed", Answer::Simple, |c| c.unsubscribe_cov(
        &c.resolve_device(REMOTE).await.unwrap(),
        9,
        ai1()
    ));
}

fn subscription() -> CovPropertySubscription {
    CovPropertySubscription {
        subscriber_process_identifier: 9,
        monitored_object_identifier: ai1(),
        monitored_property_identifier: PV,
        monitored_property_array_index: None,
        confirmed: false,
        lifetime: NonZeroU32::new(120).unwrap(),
        cov_increment: Some(0.5),
    }
}

#[tokio::test]
async fn subscribe_cov_property() {
    pin!("covp/local", Answer::Simple, |c| c
        .subscribe_cov_property(PEER, subscription()));
    pin!("covp/local", Answer::Simple, |c| c.subscribe_cov_property(
        &c.resolve_device(LOCAL).await.unwrap(),
        subscription()
    ));
    pin!("covp/routed", Answer::Simple, |c| c.subscribe_cov_property(
        &c.resolve_device(REMOTE).await.unwrap(),
        subscription()
    ));
    pin!("covp-cancel/local", Answer::Simple, |c| c
        .unsubscribe_cov_property(PEER, 9, ai1(), PV, Some(2)));
    pin!("covp-cancel/local", Answer::Simple, |c| c
        .unsubscribe_cov_property(
            &c.resolve_device(LOCAL).await.unwrap(),
            9,
            ai1(),
            PV,
            Some(2)
        ));
    pin!("covp-cancel/routed", Answer::Simple, |c| c
        .unsubscribe_cov_property(
            &c.resolve_device(REMOTE).await.unwrap(),
            9,
            ai1(),
            PV,
            Some(2)
        ));
}

pub(super) fn notification() -> AuditNotificationRequest {
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

#[tokio::test]
async fn audit_services() {
    let note = notification();
    pin!("audit-note/local", Answer::Simple, |c| c
        .confirmed_audit_notification(PEER, &note));
    pin!("uaudit-note/local", |c| c
        .unconfirmed_audit_notification(PEER, &note));
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
    pin!("audit-query/local", Answer::Error, |c| c
        .audit_log_query(PEER, &query));
}
