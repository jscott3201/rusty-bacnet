//! BUFFER_READY intrinsic reporting for the log objects (Clause 13.3.7).
//!
//! A Trend Log, Event Log or Trend Log Multiple that reports intrinsically
//! tells the recipients of its Notification Class each time
//! Notification_Threshold more records have been collected since its last
//! report, so they can fetch the new ones with ReadRange. The three tables
//! (12-29, 12-31 and 12-35) give each log the same event rows, and the
//! crate's `BufferReadyReporting` keeps them for any of the three: it runs
//! the algorithm on the log's Total_Record_Count, its pMonitoredValue, and
//! serves and takes the rows itself. [`BufferReadyReport`] carries one
//! report's counts to the server.
//!
//! Event_State never leaves NORMAL. Each report is a NORMAL to NORMAL
//! transition, so it stamps the TO_NORMAL slots of Event_Time_Stamps,
//! Event_Message_Texts and Acked_Transitions, and Event_Enable's TO_NORMAL
//! flag decides whether it goes out.
//!
//! An object adopts it in three steps: hold one and lend it to its
//! `LogLifecycle`, which restarts Records_Since_Notification at a purge;
//! offer each read and write to its `read` and `write`; and invoke
//! `impl_buffer_ready_reporting!` with the field and the log buffer.

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, EventType, NotifyType, PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

use crate::common;
use crate::event::history::{EventHistory, EventTransitionState};
use crate::event::options::REPORTING_OPTION_METADATA as OPTIONS;
use crate::event::{
    EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use crate::property_metadata::{
    PropertyConformance::Optional,
    PropertyMetadata,
    PropertyPresenceCondition::{IntrinsicReportingOptional, IntrinsicReportingRequired},
    PropertyWriteCapability::{Always, ReadOnly},
};

/// The counts one BUFFER_READY report carries (Clause 13.3.7), as the
/// server projects them into the notification's event values.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferReadyReport {
    /// Previous_Notification: Last_Notify_Record before this report, the
    /// Total_Record_Count at the report before or when detection started.
    pub previous_notification: u32,
    /// Current_Notification: the Total_Record_Count this report was made at.
    pub current_notification: u32,
}

/// A row footnote 3 of Table 12-31 (4 of Tables 12-29 and 12-35) requires
/// of a log that reports intrinsically.
const fn required(
    property: P,
    write: crate::property_metadata::PropertyWriteCapability,
) -> PropertyMetadata {
    PropertyMetadata::new(property, Optional, Some(IntrinsicReportingRequired), write)
}

/// A row footnote 5 of Table 12-31 (7 of 12-29, 6 of 12-35) only lets such
/// a log have.
const fn permitted(
    property: P,
    write: crate::property_metadata::PropertyWriteCapability,
) -> PropertyMetadata {
    PropertyMetadata::new(property, Optional, Some(IntrinsicReportingOptional), write)
}

/// The event rows a log that reports intrinsically adds, in the order the
/// three log tables list them: the ones the tables require of a log with
/// intrinsic reporting, and Event_Message_Texts, Event_Message_Texts_Config
/// and the Event_Algorithm_Inhibit pair, which they only permit (#1329).
pub(crate) const BUFFER_READY_METADATA: [PropertyMetadata; 13] = [
    required(P::NOTIFICATION_THRESHOLD, Always),
    required(P::RECORDS_SINCE_NOTIFICATION, ReadOnly),
    required(P::LAST_NOTIFY_RECORD, ReadOnly),
    required(P::NOTIFICATION_CLASS, Always),
    required(P::EVENT_ENABLE, Always),
    required(P::ACKED_TRANSITIONS, ReadOnly),
    required(P::NOTIFY_TYPE, Always),
    required(P::EVENT_TIME_STAMPS, ReadOnly),
    permitted(P::EVENT_MESSAGE_TEXTS, ReadOnly),
    OPTIONS[0],
    required(P::EVENT_DETECTION_ENABLE, Always),
    OPTIONS[1],
    OPTIONS[2],
];

/// The records collected from Total_Record_Count `from` to `to`. The count
/// wraps from 2^32 - 1 to 1, never passing zero, so a smaller `to` has gone
/// round once: the second condition of the algorithm (Clause 13.3.7).
pub(crate) fn records_between(from: u32, to: u32) -> u32 {
    if to >= from {
        to - from
    } else {
        // At most 2^32 - 2: `to` is at least 1 once the count has wrapped.
        (u64::from(to) + u64::from(u32::MAX) - u64::from(from)) as u32
    }
}

/// The event state, configuration and history of one log reporting
/// BUFFER_READY.
#[derive(Debug, Clone)]
pub(crate) struct BufferReadyReporting {
    /// Notification_Threshold, pThreshold: the records that make a report.
    /// Zero, the default, makes none.
    notification_threshold: u32,
    /// Last_Notify_Record, pPreviousCount: Total_Record_Count at the last
    /// report, or when detection started.
    last_notify_record: u32,
    /// Total_Record_Count when Records_Since_Notification last restarted:
    /// at the last report, purge or start of detection.
    count_restarted_at: u32,
    notification_class: u32,
    event_enable: EventTransitionBits,
    notify_type: NotifyType,
    /// Always NORMAL: the algorithm knows no other state.
    event_state: EventState,
    acked_transitions: EventTransitionBits,
    history: EventHistory,
    /// Event_Detection_Enable: FALSE suspends the algorithm (Clause 13.2.2.1).
    event_detection_enable: bool,
    /// The counts of the last committed report, for its event values.
    report: Option<BufferReadyReport>,
}

impl Default for BufferReadyReporting {
    /// Detection on with no threshold, so nothing is reported until one is
    /// set; Notification_Class 0, every transition enabled and acknowledged,
    /// and reports sent as events rather than alarms.
    fn default() -> Self {
        Self {
            notification_threshold: 0,
            last_notify_record: 0,
            count_restarted_at: 0,
            notification_class: 0,
            event_enable: EventTransitionBits::all(),
            notify_type: NotifyType::EVENT,
            event_state: EventState::NORMAL,
            acked_transitions: EventTransitionBits::all(),
            history: EventHistory::default(),
            event_detection_enable: true,
            report: None,
        }
    }
}

impl BufferReadyReporting {
    /// Event_State as served: always NORMAL.
    pub(crate) fn event_state(&self) -> EventState {
        self.event_state
    }

    pub(crate) fn set_notification_threshold(&mut self, threshold: u32) {
        self.notification_threshold = threshold;
    }

    pub(crate) fn set_notification_class(&mut self, class: u32) {
        self.notification_class = class;
    }

    pub(crate) fn set_event_enable(&mut self, enable: EventTransitionBits) {
        self.event_enable = enable;
    }

    pub(crate) fn set_notify_type(&mut self, notify_type: NotifyType) {
        self.notify_type = notify_type;
    }

    /// Restart Records_Since_Notification at Total_Record_Count `total`, as a
    /// purge does before it records BUFFER_PURGED (Clause 12.27.14). The
    /// algorithm's own count, from Last_Notify_Record, goes on.
    pub(crate) fn restart_count(&mut self, total: u32) {
        self.count_restarted_at = total;
    }

    /// Count afresh from Total_Record_Count `total`, as a log restored or
    /// seeded at that count does (#1537): Last_Notify_Record moves to it and
    /// Records_Since_Notification reads zero, as when detection starts, so
    /// the jump to `total` is never taken for records collected.
    pub(crate) fn restart_from(&mut self, total: u32) {
        self.last_notify_record = total;
        self.count_restarted_at = total;
    }

    /// Serve one of the event rows for a log whose Total_Record_Count is
    /// `total`, or `None` for any other property.
    pub(crate) fn read(
        &self,
        property: P,
        array_index: Option<u32>,
        total: u32,
    ) -> Option<Result<PropertyValue, Error>> {
        let unsigned = |value: u32| Some(Ok(PropertyValue::Unsigned(u64::from(value))));
        let transitions = |bits: EventTransitionBits| {
            Some(Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bits.to_bacnet()],
            }))
        };
        match property {
            P::EVENT_STATE => Some(Ok(PropertyValue::Enumerated(self.event_state.to_raw()))),
            P::EVENT_DETECTION_ENABLE => {
                Some(Ok(PropertyValue::Boolean(self.event_detection_enable)))
            }
            P::NOTIFICATION_THRESHOLD => unsigned(self.notification_threshold),
            P::RECORDS_SINCE_NOTIFICATION => {
                unsigned(records_between(self.count_restarted_at, total))
            }
            P::LAST_NOTIFY_RECORD => unsigned(self.last_notify_record),
            P::NOTIFICATION_CLASS => unsigned(self.notification_class),
            P::EVENT_ENABLE => transitions(self.event_enable),
            P::ACKED_TRANSITIONS => transitions(self.acked_transitions),
            P::NOTIFY_TYPE => Some(Ok(PropertyValue::Enumerated(self.notify_type.to_raw()))),
            _ => self.history.read(property, array_index),
        }
    }

    /// Take a network write of one of the writable event rows for a log
    /// whose Total_Record_Count is `total`, or `None` for any other property.
    /// Acked_Transitions is refused with WRITE_ACCESS_DENIED, as on every
    /// intrinsic-reporting object; the other read-only rows are left to the
    /// object's metadata. Event_Message_Texts_Config and the
    /// Event_Algorithm_Inhibit pair go to the history's
    /// [`ReportingOptions`](crate::event::options::ReportingOptions).
    ///
    /// Event_Detection_Enable FALSE puts Event_State, Acked_Transitions,
    /// Event_Time_Stamps and Event_Message_Texts back to their starting
    /// values (Clause 12.27.26). TRUE again starts the algorithm afresh,
    /// counting from the current Total_Record_Count.
    pub(crate) fn write(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
        total: u32,
    ) -> Option<Result<(), Error>> {
        if let Some(result) =
            self.history
                .write(property, array_index, value, self.event_detection_enable)
        {
            return Some(result);
        }
        let result = match property {
            P::EVENT_DETECTION_ENABLE => boolean(value).map(|enable| {
                if !enable {
                    self.event_state = EventState::NORMAL;
                    self.acked_transitions = EventTransitionBits::all();
                    self.history.reset();
                } else if !self.event_detection_enable {
                    self.last_notify_record = total;
                    self.count_restarted_at = total;
                }
                self.event_detection_enable = enable;
            }),
            P::NOTIFICATION_THRESHOLD => {
                unsigned32(value).map(|threshold| self.notification_threshold = threshold)
            }
            P::NOTIFICATION_CLASS => unsigned32(value).map(|class| self.notification_class = class),
            P::EVENT_ENABLE => match value {
                PropertyValue::BitString { unused_bits, data } => {
                    common::check_fixed_width_bit_string(*unused_bits, data, 3).map(|byte| {
                        self.event_enable = EventTransitionBits::from_bacnet(&[byte]);
                    })
                }
                _ => Err(common::invalid_data_type_error()),
            },
            P::NOTIFY_TYPE => match *value {
                PropertyValue::Enumerated(raw) => {
                    let notify_type = NotifyType::from_raw(raw);
                    if NotifyType::ALL_NAMED.iter().any(|&(_, n)| n == notify_type) {
                        self.notify_type = notify_type;
                        Ok(())
                    } else {
                        Err(common::value_out_of_range_error())
                    }
                }
                _ => Err(common::invalid_data_type_error()),
            },
            P::ACKED_TRANSITIONS => Err(common::write_access_denied_error()),
            _ => return None,
        };
        Some(result)
    }

    /// The algorithm (Clause 13.3.7) for a log whose Total_Record_Count is
    /// `total`: a NORMAL to NORMAL transition once Notification_Threshold or
    /// more records have been collected since Last_Notify_Record, counting
    /// across the wrap. No threshold, detection off, or a TRUE
    /// Event_Algorithm_Inhibit, which stops a NORMAL state's re-entry
    /// (Clause 13.2.2.1), proposes nothing; the records keep counting, so a
    /// report falls due once the inhibit clears. Proposing changes nothing;
    /// the commit does.
    pub(crate) fn propose(&self, total: u32) -> Option<TransitionOutcome> {
        let due = self.event_detection_enable
            && !self.history.options.inhibited()
            && self.notification_threshold > 0
            && records_between(self.last_notify_record, total) >= self.notification_threshold;
        due.then(|| TransitionOutcome {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::NORMAL,
            },
            event_type: EventType::BUFFER_READY,
            distribute: self.event_enable.contains(EventTransitionBits::TO_NORMAL),
        })
    }

    /// Commit one report made at Total_Record_Count `total` through the
    /// shared kernel, then move Last_Notify_Record to `total` and restart
    /// Records_Since_Notification. Any target other than NORMAL is refused
    /// as unsupported: the algorithm has no other state.
    pub(crate) fn commit(
        &mut self,
        commit: EventTransitionCommit,
        total: u32,
    ) -> Result<(), EventTransitionCommitError> {
        if commit.change.to != EventState::NORMAL {
            return Err(EventTransitionCommitError::Unsupported);
        }
        EventTransitionState::new(
            &mut self.event_state,
            &mut self.acked_transitions,
            &mut self.history,
        )
        .commit(commit)?;
        self.report = Some(BufferReadyReport {
            previous_notification: self.last_notify_record,
            current_notification: total,
        });
        self.last_notify_record = total;
        self.count_restarted_at = total;
        Ok(())
    }

    /// The counts of the last committed report.
    pub(crate) fn report(&self) -> Option<BufferReadyReport> {
        self.report
    }

    /// Acknowledge the latest report stamped `timestamp`; with detection off
    /// there is nothing to acknowledge (NO_ALARM_CONFIGURED).
    pub(crate) fn acknowledge(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<EventStateChange>, Error> {
        if !self.event_detection_enable {
            return Err(common::protocol_error(
                ErrorClass::OBJECT,
                ErrorCode::NO_ALARM_CONFIGURED,
            ));
        }
        self.history.acknowledge_correlated_detailed(
            &mut self.acked_transitions,
            event_state,
            timestamp,
        )
    }

    /// The property Event_Algorithm_Inhibit follows, if any.
    pub(crate) fn inhibit_reference(&self) -> Option<BACnetObjectPropertyReference> {
        self.history.options.inhibit_reference().cloned()
    }

    /// Take the referenced value as Event_Algorithm_Inhibit; returns whether
    /// it changed.
    pub(crate) fn follow_inhibit(&mut self, inhibit: bool) -> bool {
        self.history.options.follow(inhibit)
    }

    /// What GetEnrollmentSummary reports for the log.
    pub(crate) fn enrollment_summary(&self) -> EnrollmentSummaryCapability {
        EnrollmentSummaryCapability {
            event_type: EventType::BUFFER_READY,
            last_transition: self.history.last_transition(),
        }
    }
}

fn boolean(value: &PropertyValue) -> Result<bool, Error> {
    match *value {
        PropertyValue::Boolean(value) => Ok(value),
        _ => Err(common::invalid_data_type_error()),
    }
}

fn unsigned32(value: &PropertyValue) -> Result<u32, Error> {
    match *value {
        PropertyValue::Unsigned(value) => common::u64_to_u32(value),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// Implement the `BACnetObject` intrinsic-reporting hooks of a log that
/// holds a [`BufferReadyReporting`] in `$reporting` and its records in the
/// [`LogRecordBuffer`](crate::log_buffer::LogRecordBuffer) `$buffer`.
macro_rules! impl_buffer_ready_reporting {
    ($reporting:ident, $buffer:ident) => {
        fn enrollment_summary_capability_internal(
            &self,
        ) -> Option<$crate::event::EnrollmentSummaryCapability> {
            Some(self.$reporting.enrollment_summary())
        }

        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            self.$reporting.propose(self.$buffer.total_record_count())
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            self.$reporting.propose(self.$buffer.total_record_count())
        }

        fn commit_event_transition_internal(
            &mut self,
            commit: $crate::event::EventTransitionCommit,
        ) -> Result<(), $crate::event::EventTransitionCommitError> {
            let total = self.$buffer.total_record_count();
            self.$reporting.commit(commit, total)
        }

        fn acknowledge_alarm_correlated_internal(
            &mut self,
            event_state: bacnet_types::enums::EventState,
            timestamp: &bacnet_types::primitives::BACnetTimeStamp,
        ) -> Result<(), bacnet_types::error::Error> {
            self.$reporting
                .acknowledge(event_state, timestamp)
                .map(|_| ())
        }

        fn acknowledge_alarm_correlated_detailed_internal(
            &mut self,
            event_state: bacnet_types::enums::EventState,
            timestamp: &bacnet_types::primitives::BACnetTimeStamp,
        ) -> Result<Option<$crate::event::EventStateChange>, bacnet_types::error::Error> {
            self.$reporting.acknowledge(event_state, timestamp)
        }

        fn buffer_ready_report_internal(&self) -> Option<$crate::log_reporting::BufferReadyReport> {
            self.$reporting.report()
        }

        fn event_algorithm_inhibit_reference_internal(
            &self,
        ) -> Option<bacnet_types::constructed::BACnetObjectPropertyReference> {
            self.$reporting.inhibit_reference()
        }

        fn follow_event_algorithm_inhibit_internal(&mut self, inhibit: bool) -> bool {
            self.$reporting.follow_inhibit(inhibit)
        }
    };
}

pub(crate) use impl_buffer_ready_reporting;

#[cfg(test)]
#[path = "log_reporting_tests.rs"]
mod tests;
