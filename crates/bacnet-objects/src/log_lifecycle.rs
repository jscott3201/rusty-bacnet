//! Shared non-recursive state transitions for BACnet log objects.

use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_types::bitstring::LogStatus;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

use crate::clock::{ClockFrame, ClockReader};
use crate::common::protocol_error;
use crate::log_buffer::{LogRecordBuffer, OrdinaryAdmission, ResidentLogRecord};
use crate::log_reporting::BufferReadyReporting;
use crate::log_window::LogWindow;
use crate::property_metadata::{
    PropertyConformance::{RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::Always,
};

// Capability describes the implemented write route, not whether a particular
// value, clock, or buffer state passes that route's existing validation.
pub(crate) const LOG_ENABLE_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::LOG_ENABLE, RequiredWrite, None, Always);
pub(crate) const STOP_WHEN_FULL_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::STOP_WHEN_FULL, RequiredRead, None, Always);
pub(crate) const RECORD_COUNT_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::RECORD_COUNT, RequiredWrite, None, Always);

/// The shared lifecycle of one log object: Trend Log, Event Log or Trend Log
/// Multiple.
///
/// A log collects only while Enable is TRUE and its Start_Time / Stop_Time
/// window admits the current local time; each LOG_DISABLED it records
/// reflects both. A window with both ends unspecified is always open. A
/// purge also restarts the log's Records_Since_Notification.
pub(crate) struct LogLifecycle<'a, R: ResidentLogRecord> {
    buffer: &'a mut LogRecordBuffer<R>,
    enabled: &'a mut bool,
    stop_when_full: &'a mut bool,
    clock: Option<&'a Arc<dyn ClockReader>>,
    window: &'a mut LogWindow,
    reporting: &'a mut BufferReadyReporting,
}

impl<'a, R: ResidentLogRecord> LogLifecycle<'a, R> {
    pub(crate) fn new(
        buffer: &'a mut LogRecordBuffer<R>,
        enabled: &'a mut bool,
        stop_when_full: &'a mut bool,
        clock: Option<&'a Arc<dyn ClockReader>>,
        window: &'a mut LogWindow,
        reporting: &'a mut BufferReadyReporting,
    ) -> Self {
        Self {
            buffer,
            enabled,
            stop_when_full,
            clock,
            window,
            reporting,
        }
    }

    /// Look at the window again. When it opened or closed since the last look
    /// while Enable is TRUE, record the change: LOG_DISABLED when it closed,
    /// and when it opened a clear status, or LOG_DISABLED with Enable going
    /// FALSE if that status would fill a Stop_When_Full buffer (as an Enable
    /// write does). Returns whether a record was added.
    ///
    /// A look at a bounded window needs a valid clock; without one nothing
    /// changes and the next look with a clock records what it finds. The
    /// first look after a local change only notes where the window stands.
    pub(crate) fn refresh_window(&mut self) -> bool {
        let now = self.look_time();
        self.refresh_window_at(now)
    }

    /// Set Start_Time or Stop_Time as local configuration, checked as
    /// [`LogWindow::set`] checks it. Nothing is recorded for the change
    /// itself, but the window is looked at right away, so a client's write
    /// that opens or shuts it afterwards is recorded even before the next
    /// pass. Without a valid clock the look waits for the next one.
    pub(crate) fn configure_window(
        &mut self,
        property: P,
        value: (Date, Time),
    ) -> Result<(), Error> {
        self.window.configure(property, value)?;
        self.refresh_window();
        Ok(())
    }

    /// The local time a look at the window needs, or `None` when the clock
    /// can't give one. A window open at both ends that wasn't shut at the
    /// last look can't change and needs no time, so the clock isn't read
    /// then: most logs leave both ends open, and the poller looks at each
    /// one on every pass.
    fn look_time(&self) -> Option<(Date, Time)> {
        if self.window.is_unbounded() && self.window.last() != Some(false) {
            return None;
        }
        valid_timestamp(self.clock).ok()
    }

    /// [`refresh_window`](Self::refresh_window) at the local moment `now`,
    /// which also stamps any record it adds.
    fn refresh_window_at(&mut self, now: Option<(Date, Time)>) -> bool {
        let open = match now {
            _ if self.window.is_unbounded() => true,
            Some(now) => self.window.admits(now),
            None => return false,
        };
        let changed = self.window.last().is_some_and(|last| last != open);
        if !changed || !*self.enabled {
            self.window.note(open);
            return false;
        }
        // The record needs a timestamp; without one, keep the last look so
        // the next look with a clock records the change.
        let Some(now) = now else {
            return false;
        };
        self.window.note(open);
        let status = if !open {
            LogStatus::LOG_DISABLED
        } else if *self.stop_when_full && self.buffer.next_record_would_fill_positive_capacity() {
            *self.enabled = false;
            LogStatus::LOG_DISABLED
        } else {
            LogStatus::empty()
        };
        self.insert_status(now, status);
        true
    }

    /// Whether the window admitted logging at the last look.
    fn window_open(&self) -> bool {
        self.window.is_open()
    }

    /// Admit an ordinary record. A record that would not encode is refused
    /// with its encoding error before anything changes, so every resident
    /// record can always be served.
    ///
    /// The window is judged at the record's own timestamp, so a record taken
    /// just before or after an opening or closing lands on the right side of
    /// the status that marks it: a change since the last look is recorded
    /// first, stamped with that time, and a record outside the window is then
    /// ignored as a disabled log's is. A timestamp that isn't an actual moment
    /// is judged at the clock's time instead.
    pub(crate) fn try_add_ordinary(&mut self, record: R) -> Result<OrdinaryAdmission, Error> {
        record.encode(&mut BytesMut::new())?;
        let at = Some(record.timestamp())
            .filter(|&timestamp| LogWindow::is_moment(timestamp))
            .or_else(|| self.look_time());
        self.refresh_window_at(at);
        let collecting = *self.enabled && self.window_open();
        let admission = self
            .buffer
            .admit_ordinary(record, collecting, *self.stop_when_full);
        if admission != OrdinaryAdmission::StopBeforeFull {
            return Ok(admission);
        }

        let timestamp = valid_timestamp(self.clock)?;
        *self.enabled = false;
        self.insert_status(timestamp, LogStatus::LOG_DISABLED);
        Ok(admission)
    }

    /// A write of Enable. A change records the status it gives collection:
    /// LOG_DISABLED, or a clear status unless that status would fill a
    /// Stop_When_Full buffer. While the window is shut collection stays off
    /// whatever Enable holds, so the change is kept without a record; the
    /// window's opening is recorded when it comes.
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
        self.refresh_window();
        if !self.window_open() {
            *self.enabled = requested;
            return Ok(());
        }
        if !requested {
            *self.enabled = false;
            self.insert_status(timestamp, LogStatus::LOG_DISABLED);
            return Ok(());
        }

        let status_fills =
            *self.stop_when_full && self.buffer.next_record_would_fill_positive_capacity();
        *self.enabled = !status_fills;
        let status = if status_fills {
            LogStatus::LOG_DISABLED
        } else {
            LogStatus::empty()
        };
        self.insert_status(timestamp, status);
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
        self.insert_status(timestamp, LogStatus::LOG_DISABLED);
        Ok(())
    }

    pub(crate) fn purge(&mut self) -> Result<(), Error> {
        let timestamp = valid_timestamp(self.clock)?;
        self.refresh_window();
        let status_fills = *self.stop_when_full && self.buffer.capacity() == 1;
        let disabled = !*self.enabled || status_fills || !self.window_open();

        self.buffer.clear();
        // Records_Since_Notification starts again from zero, so the
        // BUFFER_PURGED record below is the first it counts.
        self.reporting
            .restart_count(self.buffer.total_record_count());
        if status_fills {
            *self.enabled = false;
        }
        let mut status = LogStatus::BUFFER_PURGED;
        status.set(LogStatus::LOG_DISABLED, disabled);
        self.insert_status(timestamp, status);
        Ok(())
    }

    /// Restore the buffer: `records`, oldest first, the newest numbered
    /// `total_record_count` (see `LogRecordBuffer::restore` for what is
    /// refused). BUFFER_READY then counts from the restored count. Nothing
    /// is recorded for the restore itself.
    pub(crate) fn restore(
        &mut self,
        total_record_count: u32,
        records: VecDeque<R>,
    ) -> Result<(), Error> {
        let stops_when_full = *self.enabled && *self.stop_when_full;
        self.buffer
            .restore(total_record_count, records, stops_when_full)?;
        self.reporting.restart_from(total_record_count);
        Ok(())
    }

    /// Record a LOG_INTERRUPTED status stamped `at`, which has to be an
    /// actual date and time as the clock's must be for any status record
    /// (#1537). The window is looked at `at` first, as an ordinary record's
    /// is. The status also carries LOG_DISABLED while collection is off:
    /// Enable FALSE, the window shut, or the record filling a Stop_When_Full
    /// buffer, which then turns Enable FALSE, as a purge's status does.
    pub(crate) fn record_interruption(&mut self, at: (Date, Time)) -> Result<(), Error> {
        let frame = ClockFrame {
            local_date: at.0,
            local_time: at.1,
            utc_offset: 0,
            daylight_savings_status: false,
        };
        if !frame.is_valid_actual_datetime() {
            return Err(Error::OutOfRange(
                "a LOG_INTERRUPTED record needs an actual date and time".into(),
            ));
        }
        self.refresh_window_at(Some(at));
        let fills = *self.stop_when_full && self.buffer.next_record_would_fill_positive_capacity();
        let disabled = !*self.enabled || fills || !self.window_open();
        if fills {
            *self.enabled = false;
        }
        let mut status = LogStatus::LOG_INTERRUPTED;
        status.set(LogStatus::LOG_DISABLED, disabled);
        self.insert_status(at, status);
        Ok(())
    }

    fn insert_status(&mut self, timestamp: (Date, Time), status: LogStatus) {
        self.buffer
            .insert_forced(R::log_status(timestamp.0, timestamp.1, status));
    }
}

fn valid_timestamp(clock: Option<&Arc<dyn ClockReader>>) -> Result<(Date, Time), Error> {
    let frame = clock
        .and_then(|clock| clock.read_clock())
        .filter(|frame| frame.is_valid_actual_datetime())
        .ok_or_else(|| protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM))?;
    Ok((frame.local_date, frame.local_time))
}
