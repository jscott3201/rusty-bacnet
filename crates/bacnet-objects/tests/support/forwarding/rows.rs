//! One row per `BACnetObject` method: a call with fixed arguments, and its
//! answer rendered as text. Capabilities render as the address they borrow.

use super::capabilities::CAPABILITY_ROWS;
use super::fixtures::{address, noon, oid};
use super::probe::{CUSTOM, CUSTOM_LIST};
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum, LogValue,
};
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::{BACnetTimeStamp, Date, PropertyValue};

/// A row for a method that takes `&self`.
pub type Query = fn(&dyn BACnetObject) -> String;
/// A row for a method that takes `&mut self`.
pub type Command = fn(&mut dyn BACnetObject) -> String;

/// Either kind of row.
#[derive(Clone, Copy)]
pub enum Row {
    Query(Query),
    Command(Command),
}

impl Row {
    pub fn call(self, object: &mut dyn BACnetObject) -> String {
        match self {
            Self::Query(query) => query(object),
            Self::Command(command) => command(object),
        }
    }
}

/// Every row, the queries first, then the capability rows.
pub fn rows() -> impl Iterator<Item = (&'static str, Row)> {
    let queries = QUERIES
        .iter()
        .map(|&(name, query)| (name, Row::Query(query)));
    let commands = COMMANDS
        .iter()
        .map(|&(name, command)| (name, Row::Command(command)));
    let capabilities = CAPABILITY_ROWS.iter().copied();
    queries.chain(commands).chain(capabilities)
}

/// Every row a read-only view can run: the `&self` queries, and the rows of
/// capability methods it reaches through an accessor that takes `&self`.
/// DeviceReadView's tests run these; SourceReporter's run every row.
#[allow(dead_code)]
pub fn queries() -> impl Iterator<Item = (&'static str, Query)> {
    let capabilities = CAPABILITY_ROWS.iter().filter_map(|&(name, row)| match row {
        Row::Query(query) => Some((name, query)),
        Row::Command(_) => None,
    });
    QUERIES.iter().copied().chain(capabilities)
}

/// The transition commit the rows hand over.
pub fn commit() -> EventTransitionCommit {
    EventTransitionCommit {
        change: EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        },
        coordinate: EventTransition::ToOffnormal,
        ack_required: true,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        message_text: Some("probe commit".into()),
    }
}

pub fn multiple_record() -> BACnetLogMultipleRecord {
    BACnetLogMultipleRecord {
        date: record().date,
        time: noon(),
        log_data: LogData::Values(vec![LogValue::UnsignedValue(77), LogValue::NullValue]),
    }
}

pub fn record() -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        },
        time: noon(),
        log_datum: LogDatum::UnsignedValue(77),
        status_flags: None,
    }
}

pub fn event_log_record() -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        },
        time: noon(),
        log_datum: EventLogDatum::TimeChange(1.5),
    }
}

pub const QUERIES: &[(&str, Query)] = &[
    ("audit_policy", |o| {
        format!("{:?}", o.audit_policy().map(address))
    }),
    ("audit_reporter", |o| {
        format!("{:?}", o.audit_reporter().map(address))
    }),
    ("object_identifier", |o| {
        format!("{:?}", o.object_identifier())
    }),
    ("object_name", |o| o.object_name().to_owned()),
    ("read_property", |o| {
        format!("{:?}", o.read_property(CUSTOM, Some(3)))
    }),
    ("property_metadata", |o| {
        format!("{:?}", o.property_metadata())
    }),
    ("is_writable_property", |o| {
        format!(
            "{:?}",
            [CUSTOM, P::DESCRIPTION].map(|p| o.is_writable_property(p))
        )
    }),
    ("is_array_property", |o| {
        format!(
            "{:?}",
            [CUSTOM, P::PRIORITY_ARRAY].map(|p| o.is_array_property(p))
        )
    }),
    ("is_list_property", |o| {
        format!(
            "{:?}",
            [CUSTOM_LIST, P::DATE_LIST].map(|p| o.is_list_property(p))
        )
    }),
    ("command_runner", |o| {
        format!("{:?}", o.command_runner().map(address))
    }),
    ("event_reporting", |o| {
        format!("{:?}", o.event_reporting().map(address))
    }),
    ("enrollment_eval", |o| {
        format!("{:?}", o.enrollment_eval().map(address))
    }),
    ("cov", |o| format!("{:?}", o.cov().map(address))),
    ("calendar_state", |o| {
        format!("{:?}", o.calendar_state().map(address))
    }),
    ("audit_log", |o| format!("{:?}", o.audit_log().map(address))),
    ("file_configuration", |o| {
        format!("{:?}", o.file_configuration().map(address))
    }),
    ("file_storage", |o| {
        format!("{:?}", o.file_storage().map(address))
    }),
    ("log_buffer", |o| {
        format!("{:?}", o.log_buffer().map(address))
    }),
];

pub const COMMANDS: &[(&str, Command)] = &[
    ("device_authority_internal", |o| {
        format!(
            "{:?}",
            o.device_authority_internal()
                .map(|device| device.object_identifier())
        )
    }),
    ("audit_policy_mut", |o| {
        format!("{:?}", o.audit_policy_mut().map(|policy| address(policy)))
    }),
    // The probe lends its own Reporter's authority, instance 17.
    ("audit_reporter_mut", |o| {
        format!(
            "{:?}",
            o.audit_reporter_mut()
                .map(|reporter| reporter.object_identifier())
        )
    }),
    ("write_property", |o| {
        format!(
            "{:?}",
            o.write_property(CUSTOM, Some(3), PropertyValue::Unsigned(42), Some(7))
        )
    }),
    ("write_property_from", |o| {
        let origin = CommandOrigin::Local {
            owner_device: oid(ObjectType::DEVICE, 9),
            initiating_object: Some(oid(ObjectType::SCHEDULE, 4)),
        };
        format!(
            "{:?}",
            o.write_property_from(
                CUSTOM,
                Some(3),
                PropertyValue::Unsigned(42),
                Some(7),
                &origin
            )
        )
    }),
    ("timed", |o| {
        format!("{:?}", o.timed().map(|timed| address(timed)))
    }),
    ("staging_source", |o| {
        format!("{:?}", o.staging_source().map(|source| address(source)))
    }),
    ("command_runner_mut", |o| {
        format!("{:?}", o.command_runner_mut().map(|runner| address(runner)))
    }),
    ("local_input", |o| {
        format!("{:?}", o.local_input().map(|input| address(input)))
    }),
    ("access_events", |o| {
        format!("{:?}", o.access_events().map(|events| address(events)))
    }),
    ("life_safety", |o| {
        format!(
            "{:?}",
            o.life_safety().map(|life_safety| address(life_safety))
        )
    }),
    ("event_reporting_mut", |o| {
        format!(
            "{:?}",
            o.event_reporting_mut().map(|reporting| address(reporting))
        )
    }),
    ("enrollment_eval_mut", |o| {
        format!("{:?}", o.enrollment_eval_mut().map(|eval| address(eval)))
    }),
    ("reliability_eval", |o| {
        format!("{:?}", o.reliability_eval().map(|eval| address(eval)))
    }),
    ("schedule_runner", |o| {
        format!("{:?}", o.schedule_runner().map(|runner| address(runner)))
    }),
    ("audit_log_mut", |o| {
        format!("{:?}", o.audit_log_mut().map(|log| address(log)))
    }),
    ("durable_writes", |o| {
        format!("{:?}", o.durable_writes().map(|writes| address(writes)))
    }),
    ("file_configuration_mut", |o| {
        format!("{:?}", o.file_configuration_mut().map(|file| address(file)))
    }),
    ("file_storage_mut", |o| {
        format!("{:?}", o.file_storage_mut().map(|file| address(file)))
    }),
    ("log_buffer_mut", |o| {
        format!("{:?}", o.log_buffer_mut().map(|log| address(log)))
    }),
];
