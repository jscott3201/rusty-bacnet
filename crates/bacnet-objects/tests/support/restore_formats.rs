//! Every saved format, in the order the fuzz targets' selectors index:
//! its name, whether it is two slots, its sample and the object that
//! restores it. Part of `restore.rs`.

use super::{Format, Kind, samples};

/// Every format, in a fixed order the fuzz targets' selectors index.
pub const FORMATS: &[Format] = &[
    Format {
        name: "trend_log",
        two_slot: true,
        sample: samples::trend_log,
        kind: Kind::TrendLog,
    },
    Format {
        name: "trend_log_multiple",
        two_slot: true,
        sample: samples::trend_log_multiple,
        kind: Kind::TrendLogMultiple,
    },
    Format {
        name: "event_log",
        two_slot: true,
        sample: samples::event_log,
        kind: Kind::EventLog,
    },
    Format {
        name: "audit_log",
        two_slot: true,
        sample: samples::audit_log,
        kind: Kind::AuditLog,
    },
    Format {
        name: "notification_forwarder",
        two_slot: false,
        sample: samples::notification_forwarder,
        kind: Kind::NotificationForwarder,
    },
    Format {
        name: "notification_class",
        two_slot: false,
        sample: samples::notification_class,
        kind: Kind::NotificationClass,
    },
    Format {
        name: "tags",
        two_slot: false,
        sample: samples::tags,
        kind: Kind::Tags,
    },
    Format {
        name: "color",
        two_slot: false,
        sample: samples::color,
        kind: Kind::Color,
    },
    Format {
        name: "color_temperature",
        two_slot: false,
        sample: samples::color_temperature,
        kind: Kind::ColorTemperature,
    },
    Format {
        name: "access_rights",
        two_slot: false,
        sample: samples::access_rights,
        kind: Kind::AccessRights,
    },
    Format {
        name: "timer_write_config",
        two_slot: false,
        sample: samples::timer_write_config,
        kind: Kind::TimerWriteConfig,
    },
    Format {
        name: "channel_write_config",
        two_slot: false,
        sample: samples::channel_write_config,
        kind: Kind::ChannelWriteConfig,
    },
    Format {
        name: "reporting_write_config",
        two_slot: false,
        sample: samples::reporting_write_config,
        kind: Kind::ReportingWriteConfig,
    },
    // Last, so the selectors of the formats before it stay as they were.
    Format {
        name: "alert_reporting_write_config",
        two_slot: false,
        sample: samples::alert_reporting_write_config,
        kind: Kind::AlertReportingWriteConfig,
    },
    // Kind 5 around kinds 3, 2 and 4 (#2364), after the rest for the same
    // reason.
    Format {
        name: "described_timer_write_config",
        two_slot: false,
        sample: samples::described_timer_write_config,
        kind: Kind::TimerWriteConfig,
    },
    Format {
        name: "described_channel_write_config",
        two_slot: false,
        sample: samples::described_channel_write_config,
        kind: Kind::ChannelWriteConfig,
    },
    Format {
        name: "described_reporting_write_config",
        two_slot: false,
        sample: samples::described_reporting_write_config,
        kind: Kind::ReportingWriteConfig,
    },
    // The formats of their own with a written Description (#2364), after
    // the rest for the same reason: the log's schema version 3, and the
    // version 2 tags of the forwarder's, the class's and Access Rights'.
    Format {
        name: "described_trend_log",
        two_slot: true,
        sample: samples::described_trend_log,
        kind: Kind::TrendLog,
    },
    Format {
        name: "described_notification_forwarder",
        two_slot: false,
        sample: samples::described_notification_forwarder,
        kind: Kind::NotificationForwarder,
    },
    Format {
        name: "described_notification_class",
        two_slot: false,
        sample: samples::described_notification_class,
        kind: Kind::NotificationClass,
    },
    Format {
        name: "described_access_rights",
        two_slot: false,
        sample: samples::described_access_rights,
        kind: Kind::AccessRights,
    },
    // And the formats that took a Description last (#2364): the Tags
    // format's version 2, on a Lighting Output, which keeps one there, and
    // the colour output's version 3.
    Format {
        name: "described_tags",
        two_slot: false,
        sample: samples::described_tags,
        kind: Kind::DescribedTags,
    },
    Format {
        name: "described_color",
        two_slot: false,
        sample: samples::described_color,
        kind: Kind::Color,
    },
    Format {
        name: "described_color_temperature",
        two_slot: false,
        sample: samples::described_color_temperature,
        kind: Kind::ColorTemperature,
    },
    // Kind 6, an Analog Input's written event algorithm parameters after
    // its reporting settings (#2331 PR 2), plain and under kind 5; last for
    // the same reason.
    Format {
        name: "parameters_reporting_write_config",
        two_slot: false,
        sample: samples::parameters_reporting_write_config,
        kind: Kind::ReportingWriteConfig,
    },
    Format {
        name: "described_parameters_reporting_write_config",
        two_slot: false,
        sample: samples::described_parameters_reporting_write_config,
        kind: Kind::ReportingWriteConfig,
    },
    // Kind 6 with its list row, a Multi-state Input's written Alarm_Values
    // (#2331 PR 3), plain and under kind 5; last for the same reason.
    Format {
        name: "alarm_values_reporting_write_config",
        two_slot: false,
        sample: samples::alarm_values_reporting_write_config,
        kind: Kind::MultiStateReportingWriteConfig,
    },
    Format {
        name: "described_alarm_values_reporting_write_config",
        two_slot: false,
        sample: samples::described_alarm_values_reporting_write_config,
        kind: Kind::MultiStateReportingWriteConfig,
    },
    // Kind 6 with an Access Door's three list rows, plain and under kind
    // 5, and an Access Zone's Alarm_Values (#2331 PR 4): the list row read
    // under each type's own rule. Last for the same reason.
    Format {
        name: "door_lists_reporting_write_config",
        two_slot: false,
        sample: samples::door_lists_reporting_write_config,
        kind: Kind::DoorReportingWriteConfig,
    },
    Format {
        name: "described_door_lists_reporting_write_config",
        two_slot: false,
        sample: samples::described_door_lists_reporting_write_config,
        kind: Kind::DoorReportingWriteConfig,
    },
    Format {
        name: "zone_alarm_values_reporting_write_config",
        two_slot: false,
        sample: samples::zone_alarm_values_reporting_write_config,
        kind: Kind::ZoneReportingWriteConfig,
    },
    // Kind 6 with an Event Enrollment's Event_Parameters and
    // Fault_Parameters, the two rows held in their BACnet encoding, plain
    // and under kind 5 (#2331 PR 5). Last for the same reason.
    Format {
        name: "enrollment_parameters_reporting_write_config",
        two_slot: false,
        sample: samples::enrollment_parameters_reporting_write_config,
        kind: Kind::EnrollmentReportingWriteConfig,
    },
    Format {
        name: "described_enrollment_parameters_reporting_write_config",
        two_slot: false,
        sample: samples::described_enrollment_parameters_reporting_write_config,
        kind: Kind::EnrollmentReportingWriteConfig,
    },
    // Kind 6 with a Multi-state Input's State_Text beside its Alarm_Values
    // (#2414), plain and under kind 5. Last for the same reason.
    Format {
        name: "state_text_reporting_write_config",
        two_slot: false,
        sample: samples::state_text_reporting_write_config,
        kind: Kind::MultiStateReportingWriteConfig,
    },
    Format {
        name: "described_state_text_reporting_write_config",
        two_slot: false,
        sample: samples::described_state_text_reporting_write_config,
        kind: Kind::MultiStateReportingWriteConfig,
    },
    // A Load Control's shed request with Enable and a written Description
    // (#2479). Last for the same reason.
    Format {
        name: "load_control",
        two_slot: false,
        sample: samples::load_control,
        kind: Kind::LoadControl,
    },
];
