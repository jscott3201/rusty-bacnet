//! The event notifications the device builds, and those it receives, into
//! its Event Log objects (Clause 12.27). The caller owns synchronization.

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, EventLogDatum,
    EventNotificationRequest, NotificationParameters,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tracing::{debug, warn};

use super::ObjectDatabase;
use crate::device_reference::decode_reference;

impl ObjectDatabase {
    /// Record an event notification this device generated in each of its
    /// Event Log objects, as a notification record stamped with the Device
    /// clock's local date and time.
    ///
    /// The server calls this for each notification it builds once the
    /// recipient lookup has read the Notification Class. Each log's own
    /// lifecycle decides what it keeps, as for a record the application adds
    /// through `add_record`: a disabled log ignores it, and a full log drops
    /// its oldest record or, with Stop_When_Full, stops and records that
    /// instead.
    ///
    /// Two kinds go in no log, so that a report can't prompt the next without
    /// end, through the log it watches or crosswise through another log
    /// watched in turn. One is a BUFFER_READY report on an Event Log's buffer
    /// (see `reports_an_event_log_buffer`), as for received notifications. The
    /// other is any report from an Event Enrollment of this device monitoring
    /// a property of an Event Log here: whatever its algorithm, it watches
    /// something each record changes, such as Total_Record_Count, so it isn't
    /// only BUFFER_READY that would answer its own record. A Trend Log's or
    /// Trend Log Multiple's report is logged like any other notification:
    /// those logs take no notifications, so it can't count toward the next.
    ///
    /// Without a valid Device clock no record can carry its timestamp, so
    /// nothing is logged. A log that refuses the record keeps its state; the
    /// refusal goes to the trace log and the other logs still take it. The
    /// notification is stored as given, Process Identifier included: Clause
    /// 12.27 leaves that parameter of a locally generated record to the device.
    pub fn log_event_notification(&mut self, notification: &EventNotificationRequest) {
        let logs = self.find_by_type(ObjectType::EVENT_LOG);
        if logs.is_empty() {
            return;
        }
        if reports_an_event_log_buffer(notification)
            || self.enrollment_watches_an_event_log(notification.event_object_identifier)
        {
            debug!(
                event_object = %notification.event_object_identifier,
                "Report on an Event Log not logged"
            );
            return;
        }
        self.log_notification(logs, notification);
    }

    /// Whether any Event Log has opted in to the notifications the device
    /// receives. The server asks before it decodes one.
    pub fn collects_received_event_notifications(&self) -> bool {
        self.find_by_type(ObjectType::EVENT_LOG).iter().any(|oid| {
            self.get(oid)
                .is_some_and(|log| log.logs_received_event_notifications_internal())
        })
    }

    /// Whether [`log_received_event_notification`](Self::log_received_event_notification)
    /// would record `notification`, received from another device, anywhere:
    /// some Event Log has opted in, and the notification is one a log may
    /// take. The server asks before it spends any of a source's allowance.
    pub fn takes_received_event_notification(
        &self,
        notification: &EventNotificationRequest,
    ) -> bool {
        self.collects_received_event_notifications() && self.may_log_received(notification)
    }

    /// Record an event notification this device received, as it decoded, in
    /// each Event Log that has opted in with
    /// [`EventLogObject::set_log_received_notifications`](crate::event_log::EventLogObject::set_log_received_notifications),
    /// stamped with the Device clock's local date and time. Each log's
    /// lifecycle decides what it keeps, as for the device's own
    /// notifications, and each record counts toward the log's
    /// Notification_Threshold.
    ///
    /// Two kinds are kept out. A BUFFER_READY report on an Event Log's buffer
    /// in any device isn't logged (see `reports_an_event_log_buffer`), whatever
    /// object reports it: another vendor's Event Enrollment running the
    /// algorithm on its Event Log names the log only there. Logging it could
    /// prompt a report here that, logged there in turn, prompts the next. Any
    /// other notification about an Event Log is logged. And one whose
    /// Initiating Device Identifier names this device isn't logged a second
    /// time: the device logged it when it built it.
    ///
    /// This records whatever it is given. The server calls it only for a
    /// notification that decoded in full, after holding each source to a few
    /// records a second, so a flood of them can't push the device's own
    /// records out of a log faster than that bound.
    pub fn log_received_event_notification(&mut self, notification: &EventNotificationRequest) {
        let logs = self.received_notification_logs();
        if !logs.is_empty() && self.may_log_received(notification) {
            self.log_notification(logs, notification);
        }
    }

    /// The Event Logs that take received notifications.
    fn received_notification_logs(&self) -> Vec<ObjectIdentifier> {
        let mut logs = self.find_by_type(ObjectType::EVENT_LOG);
        logs.retain(|oid| {
            self.get(oid)
                .is_some_and(|log| log.logs_received_event_notifications_internal())
        });
        logs
    }

    /// Whether a received notification is one a log may take: it isn't a
    /// BUFFER_READY report on an Event Log's buffer, and doesn't claim to
    /// come from this device.
    fn may_log_received(&self, notification: &EventNotificationRequest) -> bool {
        !reports_an_event_log_buffer(notification)
            && self.local_device().identifier() != Some(notification.initiating_device_identifier)
    }

    /// Add `notification` to each of `logs` as a notification record stamped
    /// with the Device clock, which has to be valid.
    fn log_notification(
        &mut self,
        logs: Vec<ObjectIdentifier>,
        notification: &EventNotificationRequest,
    ) {
        let Some(frame) = self
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime())
        else {
            debug!(
                event_object = %notification.event_object_identifier,
                "No valid Device clock; event notification not logged"
            );
            return;
        };
        for oid in logs {
            let Some(log) = self.get_mut(&oid) else {
                continue;
            };
            let record = BACnetEventLogRecord {
                date: frame.local_date,
                time: frame.local_time,
                log_datum: EventLogDatum::Notification(notification.clone()),
            };
            match log.add_event_log_record(record) {
                Ok(()) => {}
                Err(error) if is_unsupported(&error) => {
                    debug!(log = %oid, "Event Log object takes no records");
                }
                Err(error) => {
                    warn!(log = %oid, %error, "Event Log refused an event notification record");
                }
            }
        }
    }

    /// Whether `event_object` is an Event Enrollment of this device whose
    /// Object_Property_Reference names an Event Log here.
    fn enrollment_watches_an_event_log(&self, event_object: ObjectIdentifier) -> bool {
        match event_object.object_type() {
            ObjectType::EVENT_ENROLLMENT => self
                .get(&event_object)
                .and_then(|enrollment| {
                    enrollment
                        .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                        .ok()
                })
                .and_then(|value| {
                    decode_reference::<BACnetDeviceObjectPropertyReference>(&value).ok()
                })
                .is_some_and(|reference| {
                    reference.object_identifier.object_type() == ObjectType::EVENT_LOG
                        && self.local_device().is_local(reference.device_identifier)
                }),
            _ => false,
        }
    }
}

/// Whether `notification` is a BUFFER_READY report whose Buffer_Property
/// names an Event Log, in this device or any other. No Event Log records one:
/// each record a log takes moves its Total_Record_Count, so a log taking
/// reports on a log's buffer could set off the next report, here or in the
/// other device, and that report the next, without end.
fn reports_an_event_log_buffer(notification: &EventNotificationRequest) -> bool {
    matches!(
        &notification.event_values,
        Some(NotificationParameters::BufferReady { buffer_property, .. })
            if buffer_property.object_identifier.object_type() == ObjectType::EVENT_LOG
    )
}

/// The trait default's answer: an object of type Event Log that has no
/// record insertion.
fn is_unsupported(error: &Error) -> bool {
    matches!(error, Error::Protocol { class, code }
        if *class == ErrorClass::OBJECT
            && *code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED)
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;
