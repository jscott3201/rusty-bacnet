//! Shared non-recursive state transitions for BACnet log objects.

use std::sync::Arc;

use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};

use crate::clock::ClockReader;
use crate::common::protocol_error;
use crate::log_buffer::{LogRecordBuffer, OrdinaryAdmission};

pub(crate) const LOG_DISABLED: u8 = 0b001;
pub(crate) const BUFFER_PURGED: u8 = 0b010;

pub(crate) struct LogLifecycle<'a> {
    buffer: &'a mut LogRecordBuffer,
    enabled: &'a mut bool,
    stop_when_full: &'a mut bool,
    clock: Option<&'a Arc<dyn ClockReader>>,
}

#[derive(Clone)]
pub(crate) struct LogLifecycleSnapshot {
    buffer: LogRecordBuffer,
    enabled: bool,
    stop_when_full: bool,
}

impl<'a> LogLifecycle<'a> {
    pub(crate) fn new(
        buffer: &'a mut LogRecordBuffer,
        enabled: &'a mut bool,
        stop_when_full: &'a mut bool,
        clock: Option<&'a Arc<dyn ClockReader>>,
    ) -> Self {
        Self {
            buffer,
            enabled,
            stop_when_full,
            clock,
        }
    }

    pub(crate) fn try_add_ordinary(
        &mut self,
        record: BACnetLogRecord,
    ) -> Result<OrdinaryAdmission, Error> {
        let admission = self
            .buffer
            .admit_ordinary(record, *self.enabled, *self.stop_when_full);
        if admission != OrdinaryAdmission::StopBeforeFull {
            return Ok(admission);
        }

        let timestamp = valid_timestamp(self.clock)?;
        *self.enabled = false;
        self.insert_status(timestamp, LOG_DISABLED);
        Ok(admission)
    }

    pub(crate) fn write_enable(&mut self, requested: bool) -> Result<(), Error> {
        if requested == *self.enabled {
            return Ok(());
        }
        if requested && *self.stop_when_full && self.buffer.is_full() {
            return Err(protocol_error(
                ErrorClass::OBJECT,
                ErrorCode::LOG_BUFFER_FULL,
            ));
        }

        let timestamp = valid_timestamp(self.clock)?;
        if !requested {
            *self.enabled = false;
            self.insert_status(timestamp, LOG_DISABLED);
            return Ok(());
        }

        let status_fills =
            *self.stop_when_full && self.buffer.next_record_would_fill_positive_capacity();
        *self.enabled = !status_fills;
        self.insert_status(timestamp, u8::from(status_fills) * LOG_DISABLED);
        Ok(())
    }

    pub(crate) fn write_stop_when_full(&mut self, requested: bool) -> Result<(), Error> {
        if requested == *self.stop_when_full {
            return Ok(());
        }
        if !requested {
            *self.stop_when_full = false;
            return Ok(());
        }
        if !self.buffer.is_full() {
            *self.stop_when_full = true;
            return Ok(());
        }

        let timestamp = valid_timestamp(self.clock)?;
        *self.stop_when_full = true;
        *self.enabled = false;
        self.insert_status(timestamp, LOG_DISABLED);
        Ok(())
    }

    pub(crate) fn purge(&mut self) -> Result<(), Error> {
        let timestamp = valid_timestamp(self.clock)?;
        let status_fills = *self.stop_when_full && self.buffer.capacity() == 1;
        let disabled = !*self.enabled || status_fills;

        self.buffer.clear();
        if status_fills {
            *self.enabled = false;
        }
        let bits = BUFFER_PURGED | u8::from(disabled) * LOG_DISABLED;
        self.insert_status(timestamp, bits);
        Ok(())
    }

    fn insert_status(&mut self, timestamp: (Date, Time), bits: u8) {
        self.buffer.insert_forced(BACnetLogRecord {
            date: timestamp.0,
            time: timestamp.1,
            log_datum: LogDatum::LogStatus(bits),
            status_flags: None,
        });
    }
}

impl LogLifecycleSnapshot {
    pub(crate) fn capture(buffer: &LogRecordBuffer, enabled: bool, stop_when_full: bool) -> Self {
        Self {
            buffer: buffer.clone(),
            enabled,
            stop_when_full,
        }
    }

    pub(crate) fn restore(
        self,
        buffer: &mut LogRecordBuffer,
        enabled: &mut bool,
        stop_when_full: &mut bool,
    ) {
        *buffer = self.buffer;
        *enabled = self.enabled;
        *stop_when_full = self.stop_when_full;
    }
}

fn valid_timestamp(clock: Option<&Arc<dyn ClockReader>>) -> Result<(Date, Time), Error> {
    let frame = clock
        .and_then(|clock| clock.read_clock())
        .filter(|frame| frame.is_valid_actual_datetime())
        .ok_or_else(|| protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM))?;
    Ok((frame.local_date, frame.local_time))
}
