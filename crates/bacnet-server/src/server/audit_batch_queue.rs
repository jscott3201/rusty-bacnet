//! Bounded locally-unsent target records and command fences. No transport callbacks.
use super::audit_reporter::DeliveryCompletion;
use super::event_recipient_route::ConfirmedRecipientRoute;
use super::notification_transactions::AuditFailureTicket;
use super::*;
use bacnet_objects::audit::{AuditReporterStatus, AuditSendDelay};
use bacnet_types::primitives::BACnetTimeStamp;
use std::collections::VecDeque;

pub(super) struct QueuedAudit {
    pub status: Arc<AuditReporterStatus>,
    pub completion: DeliveryCompletion,
    pub route: Arc<ConfirmedRecipientRoute>,
    pub confirmed: bool,
    pub max_apdu: u32,
    pub epoch: u64,
    pub record: BytesMut,
    pub timestamp: BACnetTimeStamp,
    pub failure: AuditFailureTicket<Arc<ConfirmedRecipientRoute>>,
    pub delay: AuditSendDelay,
    id: u64,
    charge: usize,
    deadline: tokio::time::Instant,
}
impl QueuedAudit {
    pub(super) fn new(
        completion: DeliveryCompletion,
        record: BytesMut,
        timestamp: BACnetTimeStamp,
        failure: AuditFailureTicket<Arc<ConfirmedRecipientRoute>>,
        delay: AuditSendDelay,
    ) -> Self {
        let context = &failure.context;
        let charge = record.len() + if context.confirmed { 6 } else { 4 };
        Self {
            status: Arc::clone(&context.status),
            completion,
            route: Arc::clone(&context.route),
            confirmed: context.confirmed,
            max_apdu: context.max_apdu,
            epoch: context.epoch,
            record,
            timestamp,
            failure,
            delay,
            id: 0,
            charge,
            deadline: tokio::time::Instant::now(),
        }
    }
    fn homogeneous(&self, other: &Self) -> bool {
        self.epoch == other.epoch
            && self.route == other.route
            && self.confirmed == other.confirmed
            && self.max_apdu == other.max_apdu
    }
}
struct Lane {
    status: Arc<AuditReporterStatus>,
    records: VecDeque<QueuedAudit>,
    inflight: Vec<(u64, usize)>,
    bytes: usize,
}
struct State {
    lanes: Vec<Lane>,
    ordinal: u64,
    count: usize,
    bytes: usize,
    stop: Option<tokio::time::Instant>,
    closed: bool,
    canceled: u64,
}
pub(super) struct AuditBatchQueue {
    state: std::sync::Mutex<State>,
    pub(super) changed: tokio::sync::Notify,
    pub(super) finished: tokio::sync::Notify,
}
impl AuditBatchQueue {
    pub(super) fn new(association: &bacnet_objects::audit::TargetAuditAssociation) -> Arc<Self> {
        Arc::new(Self {
            state: std::sync::Mutex::new(State {
                lanes: association
                    .reporters()
                    .iter()
                    .map(|(_, status)| Lane {
                        status: Arc::clone(status),
                        records: VecDeque::with_capacity(64),
                        inflight: Vec::with_capacity(64),
                        bytes: 0,
                    })
                    .collect(),
                ordinal: 0,
                count: 0,
                bytes: 0,
                stop: None,
                closed: false,
                canceled: 0,
            }),
            changed: tokio::sync::Notify::new(),
            finished: tokio::sync::Notify::new(),
        })
    }
    /// Refuse known local resource pressure; the caller retains the captured loss ticket.
    pub(super) fn enqueue(&self, mut record: QueuedAudit) -> Result<(), ()> {
        let mut state = self.state.lock().unwrap();
        let Some(index) = state
            .lanes
            .iter()
            .position(|lane| Arc::ptr_eq(&lane.status, &record.status))
        else {
            return Err(());
        };
        let lane = &state.lanes[index];
        if state.closed
            || state.stop.is_some()
            || state.count == 256
            || state.bytes + record.charge > 256 * 1024
            || lane.records.len() + lane.inflight.len() == 64
            || lane.bytes + record.charge > 64 * 1024
            || record.charge > record.max_apdu as usize
        {
            return Err(());
        }
        let Some(id) = state.ordinal.checked_add(1) else {
            return Err(());
        };
        state.ordinal = id;
        record.id = id;
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(u64::from(record.delay.seconds()));
        record.deadline = state.lanes[index]
            .records
            .front()
            .map_or(deadline, |first| first.deadline.min(deadline));
        state.count += 1;
        state.bytes += record.charge;
        state.lanes[index].bytes += record.charge;
        state.lanes[index].records.push_back(record);
        drop(state);
        self.changed.notify_one();
        Ok(())
    }
    /// Hold queue ordering through all fallible preparation and the command assignment.
    pub(super) fn command<R>(
        &self,
        status: &Arc<AuditReporterStatus>,
        value: bool,
        captured: bool,
        prepare: impl FnOnce() -> Result<R, Error>,
    ) -> Result<R, Error> {
        let mut state = self.state.lock().unwrap();
        if state.closed || state.stop.is_some() {
            return Err(super::audit_recipient::denied());
        }
        let lane = state
            .lanes
            .iter_mut()
            .find(|lane| Arc::ptr_eq(&lane.status, status))
            .expect("owned Reporter");
        let fence = lane
            .records
            .back()
            .map(|r| r.id)
            .into_iter()
            .chain(lane.inflight.iter().map(|(id, _)| *id))
            .max()
            .unwrap_or(0);
        status.validate_send_now(captured)?;
        let prepared = prepare()?;
        status.commit_send_now(value, fence, captured)?;
        if value {
            let now = tokio::time::Instant::now();
            for record in &mut lane.records {
                record.deadline = record.deadline.min(now);
            }
        }
        drop(state);
        self.changed.notify_one();
        Ok(prepared)
    }
    /// A smaller live delay may accelerate, never postpone, already retained records.
    pub(super) fn delay_changed(&self, status: &Arc<AuditReporterStatus>, delay: AuditSendDelay) {
        let mut state = self.state.lock().unwrap();
        let lane = state
            .lanes
            .iter_mut()
            .find(|lane| Arc::ptr_eq(&lane.status, status))
            .expect("owned Reporter");
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(u64::from(delay.seconds()));
        for record in &mut lane.records {
            record.deadline = record.deadline.min(deadline);
        }
        drop(state);
        self.changed.notify_one();
    }
    pub(super) fn begin_stop(&self) -> tokio::time::Instant {
        let mut state = self.state.lock().unwrap();
        let now = tokio::time::Instant::now();
        let deadline = *state.stop.get_or_insert(now + Duration::from_secs(3));
        for lane in &mut state.lanes {
            for record in &mut lane.records {
                record.deadline = now;
            }
        }
        drop(state);
        self.changed.notify_one();
        deadline
    }
    pub(super) fn stopped(&self) -> bool {
        self.state.lock().unwrap().closed
    }
    pub(super) fn stop_deadline(&self) -> Option<tokio::time::Instant> {
        self.state.lock().unwrap().stop
    }
    pub(super) fn next_deadline(&self) -> Option<tokio::time::Instant> {
        let state = self.state.lock().unwrap();
        state
            .lanes
            .iter()
            .filter(|lane| lane.inflight.is_empty())
            .filter_map(|lane| lane.records.front().map(|r| r.deadline))
            .chain(state.stop)
            .min()
    }
    pub(super) fn take_due(&self) -> Option<Vec<QueuedAudit>> {
        let mut state = self.state.lock().unwrap();
        let now = tokio::time::Instant::now();
        let lane = state.lanes.iter_mut().find(|lane| {
            lane.inflight.is_empty() && lane.records.front().is_some_and(|r| r.deadline <= now)
        })?;
        let first = lane.records.pop_front().unwrap();
        let mut size = first.charge;
        let mut records = Vec::with_capacity(64);
        lane.inflight.push((first.id, first.charge));
        records.push(first);
        while lane.records.front().is_some_and(|next| {
            records[0].homogeneous(next) && size + next.record.len() <= next.max_apdu as usize
        }) {
            let next = lane.records.pop_front().unwrap();
            size += next.record.len();
            lane.inflight.push((next.id, next.charge));
            records.push(next);
        }
        Some(records)
    }
    pub(super) fn retire_local(&self, status: &Arc<AuditReporterStatus>, ids: &[u64]) {
        let mut state = self.state.lock().unwrap();
        let lane = state
            .lanes
            .iter_mut()
            .find(|lane| Arc::ptr_eq(&lane.status, status))
            .expect("owned Reporter");
        let mut count = 0;
        let mut bytes = 0;
        lane.inflight.retain(|(id, charge)| {
            if ids.contains(id) {
                count += 1;
                bytes += charge;
                false
            } else {
                true
            }
        });
        lane.bytes -= bytes;
        let first = lane
            .records
            .front()
            .map(|r| r.id)
            .into_iter()
            .chain(lane.inflight.iter().map(|(id, _)| *id))
            .min();
        status.settle_send_now(first);
        state.count -= count;
        state.bytes -= bytes;
        drop(state);
        self.changed.notify_one();
    }
    pub(super) fn close(&self) -> Vec<QueuedAudit> {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        let records = state
            .lanes
            .iter_mut()
            .flat_map(|lane| lane.records.drain(..))
            .collect::<Vec<_>>();
        state.canceled = state.canceled.saturating_add(records.len() as u64);
        for record in &records {
            let lane = state
                .lanes
                .iter_mut()
                .find(|lane| Arc::ptr_eq(&lane.status, &record.status))
                .unwrap();
            lane.bytes -= record.charge;
            state.count -= 1;
            state.bytes -= record.charge;
        }
        for lane in &state.lanes {
            lane.status.settle_send_now(None);
        }
        drop(state);
        self.finished.notify_waiters();
        records
    }
    #[cfg(test)]
    pub(super) fn resources(&self) -> (usize, usize, u64) {
        let state = self.state.lock().unwrap();
        (state.count, state.bytes, state.canceled)
    }
    pub(super) fn empty(&self) -> bool {
        self.state.lock().unwrap().count == 0
    }
}

pub(super) struct LocalDisposition {
    queue: Arc<AuditBatchQueue>,
    status: Arc<AuditReporterStatus>,
    ids: Vec<u64>,
}
impl LocalDisposition {
    pub(super) fn new(queue: Arc<AuditBatchQueue>, records: &[QueuedAudit]) -> Self {
        Self {
            queue,
            status: Arc::clone(&records[0].status),
            ids: records.iter().map(|r| r.id).collect(),
        }
    }
}
impl Drop for LocalDisposition {
    fn drop(&mut self) {
        self.queue.retire_local(&self.status, &self.ids);
    }
}

#[cfg(test)]
#[path = "audit_batch_budget_tests.rs"]
mod tests;
