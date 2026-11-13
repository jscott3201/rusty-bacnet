//! The probe's log buffer, logged under `LogBuffer::`: every method answers
//! unlike its default.

use super::{Error, Probe, noon, own_record_error};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::log_buffer::{LogBufferRecords, LogRecordIdentity};
use bacnet_objects::traits::{BACnetObject, LogBuffer, NotificationLogger};
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventNotificationRequest,
};
use bacnet_types::primitives::{Date, ObjectIdentifier};

impl LogBuffer for Probe {
    fn record_identities(&self) -> Vec<LogRecordIdentity> {
        self.called("LogBuffer::record_identities", ());
        let date = Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        };
        vec![LogRecordIdentity::new(9, date, noon()).unwrap()]
    }
    fn buffer_records(&self) -> Option<&dyn LogBufferRecords> {
        self.called("LogBuffer::buffer_records", ());
        self.trend_log.log_buffer()?.buffer_records()
    }
    fn add_trend_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        self.called("LogBuffer::add_trend_record", (record,));
        Err(own_record_error())
    }
    fn add_trend_multiple_record(&mut self, record: BACnetLogMultipleRecord) -> Result<(), Error> {
        self.called("LogBuffer::add_trend_multiple_record", (record,));
        Err(own_record_error())
    }
    fn add_event_log_record(&mut self, record: BACnetEventLogRecord) -> Result<(), Error> {
        self.called("LogBuffer::add_event_log_record", (record,));
        Err(own_record_error())
    }
    fn refresh_window(&mut self) -> bool {
        self.called("LogBuffer::refresh_window", ());
        true
    }
    fn notification_logger(&self) -> NotificationLogger {
        self.called("LogBuffer::notification_logger", ());
        logs_nothing
    }
}

/// The probe's notification logger, unlike the default's.
fn logs_nothing(_: &mut ObjectDatabase, _: Vec<ObjectIdentifier>, _: &EventNotificationRequest) {}
