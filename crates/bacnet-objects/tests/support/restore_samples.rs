//! A sample of each saved format, written through its file backend at the
//! path given: small, but with every part of its format present. Part of
//! `restore.rs`.

use std::path::Path;

use bacnet_objects::access_control::{
    AccessRightsPersistence, AccessRightsSnapshot, FileAccessRightsPersistence,
};
use bacnet_objects::audit::{
    AuditLogPersistence, AuditLogSnapshot, CompletedAuditReceipt, FileAuditLogPersistence,
};
use bacnet_objects::color::{
    ColorOutputPersistence, ColorOutputSnapshot, ColorOutputValue, FileColorOutputPersistence,
};
use bacnet_objects::log_persistence::{
    FileLogPersistence, LogAcquisitionSettings, LogPersistence, LogReportingSettings, LogSnapshot,
};
use bacnet_objects::notification_class::{
    FileNotificationClassPersistence, NotificationClassPersistence, NotificationClassSnapshot,
};
use bacnet_objects::notification_forwarder::{
    FileNotificationForwarderPersistence, ForwarderSnapshot, NotificationForwarderPersistence,
};
use bacnet_objects::object_profile::{FileTagsPersistence, TagsPersistence, TagsSnapshot};
use bacnet_objects::write_config::{
    ChannelMembers, ChannelWriteConfig, FileWriteConfigPersistence, ReportingWriteConfig,
    TimerStateChange, TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
};
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits, LogStatus};
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
    BACnetDestination, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetEventLogRecord, BACnetEventNotificationSubscription, BACnetLogMultipleRecord,
    BACnetLogRecord, BACnetNameValue, BACnetObjectPropertyReference, BACnetRecipient,
    BACnetTimerStateChangeValue, BACnetXyColor, EventLogDatum, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{LoggingType, NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use super::oid;

pub fn trend_log(base: &Path) {
    commit_log(base, trend_snapshot());
}

pub fn trend_log_multiple(base: &Path) {
    commit_log(base, multiple_snapshot());
}

pub fn event_log(base: &Path) {
    commit_log(base, event_snapshot());
}

pub fn audit_log(base: &Path) {
    FileAuditLogPersistence::new(base)
        .unwrap()
        .commit(&audit_snapshot())
        .expect("the sample commits");
}

pub fn notification_forwarder(path: &Path) {
    FileNotificationForwarderPersistence::new(path)
        .unwrap()
        .save(
            oid(ObjectType::NOTIFICATION_FORWARDER),
            &forwarder_snapshot(),
        )
        .expect("the sample saves");
}

pub fn notification_class(path: &Path) {
    let snapshot = NotificationClassSnapshot {
        recipient_list: Some(destinations()),
    };
    FileNotificationClassPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::NOTIFICATION_CLASS), &snapshot)
        .expect("the sample saves");
}

pub fn tags(path: &Path) {
    let snapshot = TagsSnapshot {
        tags: Some(vec![
            BACnetNameValue::semantic("exhaust"),
            BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
            BACnetNameValue::valued("gain", PropertyValue::Real(0.5)),
        ]),
    };
    FileTagsPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::ANALOG_INPUT), &snapshot)
        .expect("the sample saves");
}

pub fn color(path: &Path) {
    let snapshot = ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.25, 0.5))),
        default: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.0, 0.0))),
    };
    save_color(path, ObjectType::COLOR, &snapshot);
}

pub fn color_temperature(path: &Path) {
    let snapshot = ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Kelvin(4_000)),
        default: Some(ColorOutputValue::Kelvin(2_700)),
    };
    save_color(path, ObjectType::COLOR_TEMPERATURE, &snapshot);
}

fn save_color(path: &Path, kind: ObjectType, snapshot: &ColorOutputSnapshot) {
    FileColorOutputPersistence::new(path)
        .unwrap()
        .save(oid(kind), snapshot)
        .expect("the sample saves");
}

pub fn timer_write_config(path: &Path) {
    let values = [
        BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(20.5)),
        BACnetTimerStateChangeValue::NoValue,
        BACnetTimerStateChangeValue::Primitive(PropertyValue::Null),
        BACnetTimerStateChangeValue::DateTime(DATE, UNSPECIFIED.1),
        BACnetTimerStateChangeValue::Constructed(vec![0x21, 0x07]),
        BACnetTimerStateChangeValue::NoValue,
        BACnetTimerStateChangeValue::Primitive(PropertyValue::Unsigned(9)),
    ];
    let snapshot = WriteConfigSnapshot::Timer(TimerWriteConfig {
        state_change: Some(TimerStateChange {
            values,
            references: vec![member(1, None), member(2, Some(9))],
            priority_for_writing: 8,
        }),
        default_timeout: Some(90_000),
    });
    save_write_config(path, ObjectType::TIMER, &snapshot);
}

pub fn channel_write_config(path: &Path) {
    let snapshot = WriteConfigSnapshot::Channel(ChannelWriteConfig {
        members: Some(ChannelMembers {
            references: vec![member(1, None), member(2, Some(9))],
            execution_delay: vec![0, 250],
        }),
        channel_number: Some(12),
        control_groups: Some(vec![3, 0]),
        allow_group_delay_inhibit: Some(true),
    });
    save_write_config(path, ObjectType::CHANNEL, &snapshot);
}

/// An Analog Input's written reporting settings, every one set (#2331).
pub fn reporting_write_config(path: &Path) {
    let mut settings = ReportingWriteConfig::default();
    settings.notification_class = Some(7);
    settings.event_enable =
        Some(EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL);
    settings.notify_type = Some(NotifyType::EVENT);
    settings.event_detection_enable = Some(false);
    settings.event_message_texts_config = Some(["high".into(), String::new(), "normal".into()]);
    settings.event_algorithm_inhibit_ref = Some(BACnetObjectPropertyReference::new(
        oid(ObjectType::BINARY_VALUE),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    settings.event_algorithm_inhibit = Some(true);
    let snapshot = WriteConfigSnapshot::Reporting(settings);
    save_write_config(path, ObjectType::ANALOG_INPUT, &snapshot);
}

/// An Alert Enrollment's written reporting settings: a class, Event_Enable,
/// a Notify_Type of EVENT and detection off (#2331).
pub fn alert_reporting_write_config(path: &Path) {
    let mut settings = ReportingWriteConfig::default();
    settings.notification_class = Some(3);
    settings.event_enable = Some(EventTransitionBits::TO_FAULT);
    settings.notify_type = Some(NotifyType::EVENT);
    settings.event_detection_enable = Some(false);
    let snapshot = WriteConfigSnapshot::Reporting(settings);
    save_write_config(path, ObjectType::ALERT_ENROLLMENT, &snapshot);
}

fn save_write_config(path: &Path, kind: ObjectType, snapshot: &WriteConfigSnapshot) {
    FileWriteConfigPersistence::new(path)
        .unwrap()
        .save(oid(kind), snapshot)
        .expect("the sample saves");
}

/// A member or reference naming Present_Value of an Analog Output, in
/// `device` if given.
fn member(instance: u32, device: Option<u32>) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: (instance == 2).then_some(1),
        device_identifier: device
            .map(|device| ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap()),
    }
}

pub fn access_rights(path: &Path) {
    FileAccessRightsPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::ACCESS_RIGHTS), &access_rights_snapshot())
        .expect("the sample saves");
}

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 20,
    day_of_week: 2,
};

const UNSPECIFIED: (Date, Time) = (
    Date {
        year: 255,
        month: 255,
        day: 255,
        day_of_week: 255,
    },
    Time {
        hour: 255,
        minute: 255,
        second: 255,
        hundredths: 255,
    },
);

fn at(hour: u8) -> Time {
    Time {
        hour,
        minute: 15,
        second: 0,
        hundredths: 0,
    }
}

fn reference(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

fn commit_log<R>(base: &Path, snapshot: LogSnapshot<R>)
where
    FileLogPersistence: LogPersistence<R>,
{
    FileLogPersistence::new(base)
        .unwrap()
        .commit(&snapshot)
        .expect("the sample commits");
}

fn log_snapshot<R>(kind: ObjectType, records: Vec<R>) -> LogSnapshot<R> {
    LogSnapshot {
        object_identifier: oid(kind),
        generation: 3,
        buffer_size: 4,
        log_enable: true,
        stop_when_full: false,
        total_record_count: 7,
        records,
        last_notify_record: 5,
        records_since_notification_from: 6,
        notification_threshold: 4,
        start_time: (DATE, at(1)),
        stop_time: UNSPECIFIED,
        acquisition: Some(LogAcquisitionSettings {
            logging_type: LoggingType::POLLED,
            log_interval: 500,
            align_intervals: true,
            interval_offset: 20,
        }),
        log_device_object_property: vec![reference(1)],
        reporting: written_reporting(),
    }
}

/// Every reporting setting every log keeps, as writes set them (#1952):
/// 43 octets in a slot.
fn written_reporting() -> LogReportingSettings {
    LogReportingSettings {
        notification_class: Some(7),
        event_enable: Some(EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL),
        notify_type: Some(NotifyType::ALARM),
        event_detection_enable: Some(false),
        event_message_texts_config: Some(["off".into(), String::new(), "back".into()]),
        event_algorithm_inhibit_ref: Some(BACnetObjectPropertyReference::new(
            oid(ObjectType::BINARY_VALUE),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )),
        event_algorithm_inhibit: Some(true),
        client_cov_increment: None,
        cov_resubscription_interval: None,
    }
}

fn trend_snapshot() -> LogSnapshot<BACnetLogRecord> {
    let record = |hour: u8, log_datum| BACnetLogRecord {
        date: DATE,
        time: at(hour),
        log_datum,
        status_flags: None,
    };
    let mut snapshot = log_snapshot(
        ObjectType::TREND_LOG,
        vec![
            record(1, LogDatum::RealValue(1.5)),
            record(2, LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED)),
            record(3, LogDatum::UnsignedValue(9)),
        ],
    );
    // A Trend Log's two COV settings too: 9 octets more.
    snapshot.reporting.client_cov_increment = Some(Some(0.5));
    snapshot.reporting.cov_resubscription_interval = Some(600);
    snapshot
}

fn multiple_snapshot() -> LogSnapshot<BACnetLogMultipleRecord> {
    let mut snapshot = log_snapshot(
        ObjectType::TREND_LOG_MULTIPLE,
        (1..3)
            .map(|hour| BACnetLogMultipleRecord {
                date: DATE,
                time: at(hour),
                log_data: LogData::Values(vec![
                    LogValue::UnsignedValue(u64::from(hour)),
                    LogValue::BooleanValue(hour % 2 == 0),
                ]),
            })
            .collect(),
    );
    snapshot.log_device_object_property.push(reference(2));
    snapshot
}

fn event_snapshot() -> LogSnapshot<BACnetEventLogRecord> {
    let mut snapshot = log_snapshot(
        ObjectType::EVENT_LOG,
        (1..3)
            .map(|hour| BACnetEventLogRecord {
                date: DATE,
                time: at(hour),
                log_datum: EventLogDatum::TimeChange(f32::from(hour)),
            })
            .collect(),
    );
    snapshot.acquisition = None;
    snapshot.log_device_object_property.clear();
    snapshot
}

fn audit_snapshot() -> AuditLogSnapshot {
    let record = |sequence_number, datum| BACnetAuditLogRecordResult {
        sequence_number,
        record: BACnetAuditLogRecord {
            timestamp: (DATE, at(12)),
            datum,
        },
    };
    AuditLogSnapshot {
        object_identifier: oid(ObjectType::AUDIT_LOG),
        generation: 3,
        capacity: 4,
        log_enable: true,
        total_record_count: 2,
        records: vec![
            record(1, BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED)),
            record(2, BACnetAuditLogDatum::TimeChange(-1.5)),
        ],
        completed_receipts: vec![
            CompletedAuditReceipt::new(vec![1, 2, 3], 1_700_000_000_000).expect("a short key"),
        ],
    }
}

fn device(instance: u32) -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap())
}

fn destinations() -> Vec<BACnetDestination> {
    vec![BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: device(7),
        process_identifier: 3,
        issue_confirmed_notifications: true,
        transitions: EventTransitionBits::all(),
    }]
}

fn forwarder_snapshot() -> ForwarderSnapshot {
    ForwarderSnapshot {
        recipient_list: Some(destinations()),
        subscribed_recipients: vec![BACnetEventNotificationSubscription {
            recipient: device(8),
            process_identifier: 1,
            issue_confirmed_notifications: false,
            time_remaining: 30,
        }],
    }
}

fn access_rights_snapshot() -> AccessRightsSnapshot {
    let location = BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_POINT, 2).unwrap(),
    };
    AccessRightsSnapshot {
        positive_access_rules: Some(vec![BACnetAccessRule::new(None, Some(location), true)]),
        negative_access_rules: Some(Vec::new()),
        enable: Some(true),
        accompaniment: None,
    }
}
