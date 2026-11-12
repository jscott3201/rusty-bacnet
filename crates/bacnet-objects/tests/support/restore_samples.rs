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
use bacnet_objects::load_control::{
    FileLoadControlPersistence, LoadControlPersistence, LoadControlSnapshot,
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
    AlgorithmWriteConfig, ChannelMembers, ChannelWriteConfig, FileWriteConfigPersistence,
    ReportingWriteConfig, TimerStateChange, TimerWriteConfig, WriteConfigPersistence,
    WriteConfigSnapshot,
};
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits, LimitEnable, LogStatus};
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

/// A Trend Log with a written Description: schema version 3 (#2364).
pub fn described_trend_log(base: &Path) {
    let mut snapshot = trend_snapshot();
    snapshot.reporting.description = Some("Lobby".into());
    commit_log(base, snapshot);
}

pub fn audit_log(base: &Path) {
    FileAuditLogPersistence::new(base)
        .unwrap()
        .commit(&audit_snapshot())
        .expect("the sample commits");
}

pub fn notification_forwarder(path: &Path) {
    save_forwarder(path, forwarder_snapshot());
}

/// A forwarder with a written Description: `RBNFWD02` (#2364).
pub fn described_notification_forwarder(path: &Path) {
    let mut snapshot = forwarder_snapshot();
    snapshot.description = Some("Lobby".into());
    save_forwarder(path, snapshot);
}

fn save_forwarder(path: &Path, snapshot: ForwarderSnapshot) {
    FileNotificationForwarderPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::NOTIFICATION_FORWARDER), &snapshot)
        .expect("the sample saves");
}

/// A Load Control shedding a percent short of compliance, with a written
/// Description: every field of `RBNLCT01` present (#2479).
pub fn load_control(path: &Path) {
    save_load_control(path, Vec::new());
}

/// [`load_control`] with two written Shed_Levels elements: `RBNLCT02`
/// (#2502).
pub fn written_levels_load_control(path: &Path) {
    save_load_control(path, vec![(1, 3), (3, 9)]);
}

fn save_load_control(path: &Path, shed_levels: Vec<(u32, u64)>) {
    // Built from its default: the snapshot is non-exhaustive (#2502).
    let mut snapshot = LoadControlSnapshot::default();
    snapshot.state = bacnet_types::enums::ShedState::SHED_NON_COMPLIANT;
    snapshot.requested_shed_level = bacnet_types::constructed::BACnetShedLevel::Percent(80);
    snapshot.start_time = (
        Date {
            year: 126,
            month: 11,
            day: 11,
            day_of_week: 3,
        },
        Time {
            hour: 10,
            minute: 5,
            second: 0,
            hundredths: 0,
        },
    );
    snapshot.shed_duration = 30;
    snapshot.duty_window = 10;
    snapshot.agreed_duty_window = 15;
    snapshot.enable = true;
    snapshot.description = Some("Chillers".into());
    snapshot.shed_levels = shed_levels;
    FileLoadControlPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::LOAD_CONTROL), &snapshot)
        .expect("the sample saves");
}

pub fn notification_class(path: &Path) {
    save_class(path, None);
}

/// A Notification Class with a written Description: `RBNNCL02` (#2364).
pub fn described_notification_class(path: &Path) {
    save_class(path, Some("Lobby"));
}

fn save_class(path: &Path, description: Option<&str>) {
    let snapshot = NotificationClassSnapshot {
        recipient_list: Some(destinations()),
        description: description.map(Into::into),
    };
    FileNotificationClassPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::NOTIFICATION_CLASS), &snapshot)
        .expect("the sample saves");
}

pub fn tags(path: &Path) {
    save_tags(path, ObjectType::ANALOG_INPUT, None);
}

/// A Lighting Output's Tags with a written Description: `RBNTAG02` (#2364).
pub fn described_tags(path: &Path) {
    save_tags(path, ObjectType::LIGHTING_OUTPUT, Some("Lobby"));
}

fn save_tags(path: &Path, kind: ObjectType, description: Option<&str>) {
    let snapshot = TagsSnapshot {
        tags: Some(vec![
            BACnetNameValue::semantic("exhaust"),
            BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
            BACnetNameValue::valued("gain", PropertyValue::Real(0.5)),
        ]),
        description: description.map(Into::into),
    };
    FileTagsPersistence::new(path)
        .unwrap()
        .save(oid(kind), &snapshot)
        .expect("the sample saves");
}

pub fn color(path: &Path) {
    save_color(path, ObjectType::COLOR, &color_snapshot(None));
}

/// A Color object's output with a written Description: `RBNCLR03` (#2364).
pub fn described_color(path: &Path) {
    save_color(path, ObjectType::COLOR, &color_snapshot(Some("Lobby")));
}

fn color_snapshot(description: Option<&str>) -> ColorOutputSnapshot {
    ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.25, 0.5))),
        default: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.0, 0.0))),
        description: description.map(Into::into),
    }
}

pub fn color_temperature(path: &Path) {
    let snapshot = temperature_snapshot(None);
    save_color(path, ObjectType::COLOR_TEMPERATURE, &snapshot);
}

/// A Color Temperature object's output with a written Description (#2364).
pub fn described_color_temperature(path: &Path) {
    let snapshot = temperature_snapshot(Some("Lobby"));
    save_color(path, ObjectType::COLOR_TEMPERATURE, &snapshot);
}

fn temperature_snapshot(description: Option<&str>) -> ColorOutputSnapshot {
    ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Kelvin(4_000)),
        default: Some(ColorOutputValue::Kelvin(2_700)),
        description: description.map(Into::into),
    }
}

fn save_color(path: &Path, kind: ObjectType, snapshot: &ColorOutputSnapshot) {
    FileColorOutputPersistence::new(path)
        .unwrap()
        .save(oid(kind), snapshot)
        .expect("the sample saves");
}

pub fn timer_write_config(path: &Path) {
    save_write_config(path, ObjectType::TIMER, &timer_config(None));
}

/// A Timer's write configuration with a written Description (#2364).
pub fn described_timer_write_config(path: &Path) {
    save_write_config(path, ObjectType::TIMER, &timer_config(Some("Lobby")));
}

fn timer_config(description: Option<&str>) -> WriteConfigSnapshot {
    let values = [
        BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(20.5)),
        BACnetTimerStateChangeValue::NoValue,
        BACnetTimerStateChangeValue::Primitive(PropertyValue::Null),
        BACnetTimerStateChangeValue::DateTime(DATE, UNSPECIFIED.1),
        BACnetTimerStateChangeValue::Constructed(vec![0x21, 0x07]),
        BACnetTimerStateChangeValue::NoValue,
        BACnetTimerStateChangeValue::Primitive(PropertyValue::Unsigned(9)),
    ];
    let mut config = TimerWriteConfig::default();
    config.state_change = Some(TimerStateChange {
        values,
        references: vec![member(1, None), member(2, Some(9))],
        priority_for_writing: 8,
    });
    config.default_timeout = Some(90_000);
    config.description = description.map(Into::into);
    WriteConfigSnapshot::Timer(config)
}

pub fn channel_write_config(path: &Path) {
    save_write_config(path, ObjectType::CHANNEL, &channel_config(None));
}

/// A Channel's write configuration with a written Description (#2364).
pub fn described_channel_write_config(path: &Path) {
    save_write_config(path, ObjectType::CHANNEL, &channel_config(Some("Lobby")));
}

fn channel_config(description: Option<&str>) -> WriteConfigSnapshot {
    let mut config = ChannelWriteConfig::default();
    config.members = Some(ChannelMembers {
        references: vec![member(1, None), member(2, Some(9))],
        execution_delay: vec![0, 250],
    });
    config.channel_number = Some(12);
    config.control_groups = Some(vec![3, 0]);
    config.allow_group_delay_inhibit = Some(true);
    config.description = description.map(Into::into);
    WriteConfigSnapshot::Channel(config)
}

/// An Analog Input's written reporting settings, every one set (#2331).
pub fn reporting_write_config(path: &Path) {
    save_write_config(path, ObjectType::ANALOG_INPUT, &reporting_config(None));
}

/// An Analog Input's written reporting settings with a written Description
/// (#2364).
pub fn described_reporting_write_config(path: &Path) {
    let snapshot = reporting_config(Some("Lobby"));
    save_write_config(path, ObjectType::ANALOG_INPUT, &snapshot);
}

/// An Analog Input's written reporting settings and every event algorithm
/// parameter it keeps (#2331 PR 2): kind 6.
pub fn parameters_reporting_write_config(path: &Path) {
    let snapshot = with_parameters(reporting_config(None));
    save_write_config(path, ObjectType::ANALOG_INPUT, &snapshot);
}

/// The same with a written Description: kind 5 around kind 6.
pub fn described_parameters_reporting_write_config(path: &Path) {
    let snapshot = with_parameters(reporting_config(Some("Lobby")));
    save_write_config(path, ObjectType::ANALOG_INPUT, &snapshot);
}

/// The states of the Multi-state Input the list samples are for.
pub const MULTI_STATE_SAMPLE_STATES: u32 = 8;

/// A Multi-state Input's written reporting settings, time delays and
/// Alarm_Values (#2331 PR 3): kind 6 with its list row.
pub fn alarm_values_reporting_write_config(path: &Path) {
    let snapshot = with_alarm_values(reporting_config(None));
    save_write_config(path, ObjectType::MULTI_STATE_INPUT, &snapshot);
}

/// The same with a written Description: kind 5 around kind 6.
pub fn described_alarm_values_reporting_write_config(path: &Path) {
    let snapshot = with_alarm_values(reporting_config(Some("Lobby")));
    save_write_config(path, ObjectType::MULTI_STATE_INPUT, &snapshot);
}

/// `snapshot`, reporting settings, with every parameter a Multi-state Input
/// keeps written: a list of three states, the last the object's highest.
fn with_alarm_values(mut snapshot: WriteConfigSnapshot) -> WriteConfigSnapshot {
    let WriteConfigSnapshot::Reporting(settings) = &mut snapshot else {
        unreachable!("reporting settings");
    };
    let mut algorithm = AlgorithmWriteConfig::default();
    algorithm.time_delay = Some(30);
    algorithm.time_delay_normal = Some(5);
    algorithm.alarm_values = Some(vec![3, 1, MULTI_STATE_SAMPLE_STATES]);
    settings.algorithm = algorithm;
    snapshot
}

/// A Multi-state Input's written reporting settings, time delays,
/// Alarm_Values and State_Text (#2414): kind 6 with the list row and the
/// labels row. The labels give the object one state more than the eight it
/// is built with, and the list names it, as a peer that adds a state and
/// lists it leaves storage.
pub fn state_text_reporting_write_config(path: &Path) {
    let snapshot = with_state_text(reporting_config(None));
    save_write_config(path, ObjectType::MULTI_STATE_INPUT, &snapshot);
}

/// The same with a written Description: kind 5 around kind 6.
pub fn described_state_text_reporting_write_config(path: &Path) {
    let snapshot = with_state_text(reporting_config(Some("Lobby")));
    save_write_config(path, ObjectType::MULTI_STATE_INPUT, &snapshot);
}

fn with_state_text(snapshot: WriteConfigSnapshot) -> WriteConfigSnapshot {
    let mut snapshot = with_alarm_values(snapshot);
    let WriteConfigSnapshot::Reporting(settings) = &mut snapshot else {
        unreachable!("reporting settings");
    };
    let states = MULTI_STATE_SAMPLE_STATES + 1;
    settings.algorithm.alarm_values = Some(vec![3, 1, states]);
    // Labels past ASCII, and the added state's left empty.
    let labels = (1..states).map(|state| format!("Stufe {state} ä"));
    settings.algorithm.state_text = Some(labels.chain([String::new()]).collect());
    snapshot
}

/// An Access Door's written reporting settings, time delays and its three
/// lists of door alarm states (#2331 PR 4): kind 6 with all three list
/// rows.
pub fn door_lists_reporting_write_config(path: &Path) {
    let snapshot = with_door_lists(reporting_config(None));
    save_write_config(path, ObjectType::ACCESS_DOOR, &snapshot);
}

/// The same with a written Description: kind 5 around kind 6.
pub fn described_door_lists_reporting_write_config(path: &Path) {
    let snapshot = with_door_lists(reporting_config(Some("Lobby")));
    save_write_config(path, ObjectType::ACCESS_DOOR, &snapshot);
}

/// An Access Zone's written reporting settings, time delays and
/// Alarm_Values (#2331 PR 4): the list row under a zone's rule.
pub fn zone_alarm_values_reporting_write_config(path: &Path) {
    let mut snapshot = reporting_config(None);
    let WriteConfigSnapshot::Reporting(settings) = &mut snapshot else {
        unreachable!("reporting settings");
    };
    let mut algorithm = AlgorithmWriteConfig::default();
    algorithm.time_delay = Some(30);
    algorithm.time_delay_normal = Some(5);
    // A named occupancy state and the two ends of the proprietary ones.
    algorithm.alarm_values = Some(vec![4, 64, 65_535]);
    settings.algorithm = algorithm;
    save_write_config(path, ObjectType::ACCESS_ZONE, &snapshot);
}

/// An Event Enrollment's written reporting settings, Time_Delay_Normal,
/// Event_Parameters and Fault_Parameters (#2331 PR 5): kind 6 with the two
/// rows that hold a value in its BACnet encoding.
pub fn enrollment_parameters_reporting_write_config(path: &Path) {
    let snapshot = with_enrollment_parameters(reporting_config(None));
    save_write_config(path, ObjectType::EVENT_ENROLLMENT, &snapshot);
}

/// The same with a written Description: kind 5 around kind 6.
pub fn described_enrollment_parameters_reporting_write_config(path: &Path) {
    let snapshot = with_enrollment_parameters(reporting_config(Some("Lobby")));
    save_write_config(path, ObjectType::EVENT_ENROLLMENT, &snapshot);
}

/// `snapshot`, reporting settings, with every parameter an Event
/// Enrollment keeps written: change-of-state parameters with a list of
/// states, and fault-life-safety ones with a list and a reference.
fn with_enrollment_parameters(mut snapshot: WriteConfigSnapshot) -> WriteConfigSnapshot {
    use bacnet_types::constructed::{
        BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
        FaultParameters,
    };
    use bacnet_types::enums::LifeSafetyState;
    let WriteConfigSnapshot::Reporting(settings) = &mut snapshot else {
        unreachable!("reporting settings");
    };
    let mut algorithm = AlgorithmWriteConfig::default();
    algorithm.time_delay_normal = Some(5);
    algorithm.event_parameters = Some(BACnetEventParameter::ChangeOfState {
        time_delay: 30,
        list_of_values: vec![
            BACnetPropertyStates::State(3),
            BACnetPropertyStates::BooleanValue(true),
            BACnetPropertyStates::UnsignedValue(70_000),
        ],
    });
    algorithm.fault_parameters = Some(FaultParameters::FaultLifeSafety {
        fault_values: vec![LifeSafetyState::FAULT, LifeSafetyState::from_raw(300)],
        mode_property_reference: BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap(),
            PropertyIdentifier::MODE.to_raw(),
        ),
    });
    settings.algorithm = algorithm;
    snapshot
}

/// `snapshot`, reporting settings, with every parameter an Access Door
/// keeps written: named door alarm states and the two ends of the
/// proprietary ones.
fn with_door_lists(mut snapshot: WriteConfigSnapshot) -> WriteConfigSnapshot {
    let WriteConfigSnapshot::Reporting(settings) = &mut snapshot else {
        unreachable!("reporting settings");
    };
    let mut algorithm = AlgorithmWriteConfig::default();
    algorithm.time_delay = Some(30);
    algorithm.time_delay_normal = Some(5);
    algorithm.alarm_values = Some(vec![3, 4, 256]);
    algorithm.fault_values = Some(vec![5]);
    algorithm.masked_alarm_values = Some(vec![7, 65_535]);
    settings.algorithm = algorithm;
    snapshot
}

/// `snapshot`, reporting settings, with every parameter an analog object
/// keeps written.
fn with_parameters(mut snapshot: WriteConfigSnapshot) -> WriteConfigSnapshot {
    let WriteConfigSnapshot::Reporting(settings) = &mut snapshot else {
        unreachable!("reporting settings");
    };
    let mut algorithm = AlgorithmWriteConfig::default();
    algorithm.time_delay = Some(30);
    algorithm.time_delay_normal = Some(5);
    algorithm.high_limit = Some(80.5);
    algorithm.low_limit = Some(-12.25);
    algorithm.deadband = Some(1.5);
    algorithm.limit_enable = Some(LimitEnable::HIGH_LIMIT_ENABLE);
    settings.algorithm = algorithm;
    snapshot
}

fn reporting_config(description: Option<&str>) -> WriteConfigSnapshot {
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
    settings.description = description.map(Into::into);
    WriteConfigSnapshot::Reporting(settings)
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
    save_rights(path, access_rights_snapshot());
}

/// An Access Rights object with a written Description: `RBNACR02` (#2364).
pub fn described_access_rights(path: &Path) {
    let mut snapshot = access_rights_snapshot();
    snapshot.description = Some("Lobby".into());
    save_rights(path, snapshot);
}

fn save_rights(path: &Path, snapshot: AccessRightsSnapshot) {
    FileAccessRightsPersistence::new(path)
        .unwrap()
        .save(oid(ObjectType::ACCESS_RIGHTS), &snapshot)
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
        description: None,
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
        description: None,
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
        description: None,
    }
}
