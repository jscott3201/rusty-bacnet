#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::AuditNotificationRequest;
use crate::common::MAX_DECODED_ITEMS;
use bacnet_encoding::constructed::tagged::{expect_end, misplaced_tag};
use bacnet_encoding::constructed::{decode_audit_notification_at, encode_audit_notification};
use bacnet_encoding::tags;
use bacnet_types::error::{Error, RangeError};
use bytes::BytesMut;

const NOTIFICATIONS_TAG: u8 = 0;

pub(super) fn encode(request: &AuditNotificationRequest, buf: &mut BytesMut) -> Result<(), Error> {
    if request.notifications.is_empty() || request.notifications.len() > MAX_DECODED_ITEMS {
        return Err(
            RangeError::new("AuditNotification notifications count {} is outside 1..={}")
                .with_values(request.notifications.len() as u64, MAX_DECODED_ITEMS as u64)
                .into(),
        );
    }

    let mut encoded = BytesMut::new();
    tags::encode_opening_tag(&mut encoded, NOTIFICATIONS_TAG);
    for notification in &request.notifications {
        encode_audit_notification(notification, &mut encoded)?;
    }
    tags::encode_closing_tag(&mut encoded, NOTIFICATIONS_TAG);
    buf.extend_from_slice(&encoded);
    Ok(())
}

pub(super) fn decode(data: &[u8]) -> Result<AuditNotificationRequest, Error> {
    let (outer, body_start) = tags::decode_tag(data, 0)?;
    if !outer.is_opening_tag(NOTIFICATIONS_TAG) {
        return Err(misplaced_tag(
            data,
            &outer,
            Some(NOTIFICATIONS_TAG),
            0,
            "AuditNotification expected notifications opening tag [0]",
        ));
    }
    let (body, end) = tags::extract_context_value(data, body_start, NOTIFICATIONS_TAG)?;
    expect_end(data, end, end, "AuditNotification")?;
    if body.is_empty() {
        // An empty list is outside the list's size range, as an empty COV
        // list of values is (#1446).
        return Err(Error::out_of_range(
            body_start,
            "AuditNotification notifications must not be empty",
        ));
    }

    let mut notifications = Vec::new();
    let mut offset = 0;
    while offset < body.len() {
        if notifications.len() >= MAX_DECODED_ITEMS {
            return Err(Error::overflow(
                body_start + offset,
                "AuditNotification notifications count exceeds limit",
            ));
        }
        let (notification, next) = decode_audit_notification_at(body, offset)?;
        if next <= offset {
            return Err(Error::decoding(
                body_start + offset,
                "AuditNotification decoder made no progress",
            ));
        }
        notifications.push(notification);
        offset = next;
    }

    Ok(AuditNotificationRequest { notifications })
}
