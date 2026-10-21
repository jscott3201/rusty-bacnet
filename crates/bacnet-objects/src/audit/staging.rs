//! Audit Log commits off the database lock (#1270).
//!
//! Every commit runs on the log's writer thread ([`crate::durable`]). The
//! server stages an inbound notification batch, a Log_Enable or Buffer_Size
//! write it receives, and an application's purge (#1238): the log builds the
//! next snapshot and queues its commit, goes on serving the committed state,
//! and takes the new snapshot only once the commit is durable. The server
//! awaits the commit after dropping the database guard, so readers carry on
//! meanwhile, and a confirmed batch is still acknowledged only after its
//! records and receipt are stored. A commit that fails leaves the log as it
//! was, and the request is refused.
//!
//! A request's Log_Enable and Buffer_Size writes are staged together: one
//! commit holds them all, and the log keeps the state each leaves as a step
//! the request takes when it makes that write. If the request stops before
//! taking every step, the log keeps what it took and storage is set back to
//! the state served.
//!
//! One commit is staged at a time; a request that finds one staged waits for
//! it without the guard. Code that changes the log in place (an
//! application's `add_record`, or a change nobody staged) first settles what
//! is staged: a batch whose commit ran is applied if it succeeded, its
//! outcome kept for the batch's request, and a staged change is dropped, its
//! request then committing in place.

use std::collections::VecDeque;
use std::sync::Arc;

use super::buffer::{apply_change, written_buffer_size, Timestamp};
use super::*;
use crate::durable::{
    DurableWrites, Event, PendingWrite, SaveTicket, SaveWait, SaveWriter, StageStep,
};

/// Outcomes of batches settled before their requests came back for them.
/// Past it the oldest is dropped and its request is refused with
/// OPERATIONAL_PROBLEM. That needs more than 16 batches settled while their
/// requests are all still away, which is not expected in practice.
const MAX_SETTLED: usize = 16;

/// What staging an Audit notification batch gave the caller.
#[derive(Debug)]
pub enum AuditBatchStage {
    /// No staged commit was needed: the batch's outcome, and whether its
    /// retained records changed. A duplicate confirmed batch, an unconfirmed
    /// one that changed nothing, and any batch to a sink that stores at once
    /// end here.
    Done(ConfirmedAuditNotificationOutcome, bool),
    /// The commit is queued. Await [`StagedAuditBatch::saved`] without the
    /// database guard, then pass the batch to
    /// [`AuditLogNotificationSink::finish_notification_batch`].
    Staged(StagedAuditBatch),
    /// Another staged commit holds the log. Await the wait, then stage the
    /// batch again.
    Busy(SaveWait),
}

/// A notification batch whose commit is queued on the Audit Log's writer.
#[derive(Debug)]
pub struct StagedAuditBatch {
    token: u64,
    saved: SaveWait,
}

impl StagedAuditBatch {
    /// A future that resolves once the commit has run.
    pub fn saved(&self) -> SaveWait {
        self.saved.clone()
    }
}

pub(super) enum StagedKind {
    Batch {
        token: u64,
        outcome: ConfirmedAuditNotificationOutcome,
        changed: bool,
    },
    /// The changes one request makes, in its order, all in one commit. The
    /// request makes them one at a time under the guard, and the log takes
    /// each change's step as it is made; `taken` counts the steps taken.
    Changes { steps: VecDeque<Step>, taken: usize },
}

/// One staged change and the log as it leaves it. The last step's state is
/// the snapshot committed.
pub(super) struct Step {
    change: StagedChange,
    state: Arc<AuditLogSnapshot>,
}

/// A change a request stages and then makes under the guard, where the log
/// takes the staged step if it still builds on the state served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StagedChange {
    /// A Log_Enable write of this value.
    LogEnable(bool),
    /// A Buffer_Size write of this size.
    BufferSize(u32),
    /// The application's purge.
    Purge,
}

/// The next snapshot, held aside while its commit runs.
pub(super) struct StagedCommit {
    snapshot: Arc<AuditLogSnapshot>,
    ticket: SaveTicket,
    kind: StagedKind,
    /// Set when the commit is taken or dropped, for a request that found the
    /// log busy.
    released: Arc<Event>,
}

/// A batch settled before its request finished it.
pub(super) struct Settled {
    token: u64,
    result: Result<(ConfirmedAuditNotificationOutcome, bool), Error>,
}

pub(super) fn operational_problem() -> Error {
    Error::Protocol {
        class: ErrorClass::DEVICE,
        code: ErrorCode::OPERATIONAL_PROBLEM,
    }
}

/// The writer that commits `oid`'s snapshots to `persistence`.
pub(super) fn writer(
    oid: ObjectIdentifier,
    persistence: Arc<dyn AuditLogPersistence>,
) -> SaveWriter<Arc<AuditLogSnapshot>> {
    SaveWriter::new(
        format!("bacnet-al-{}-save", oid.instance_number()),
        move |snapshot: &Arc<AuditLogSnapshot>| persistence.commit(snapshot),
        move |snapshot, result| {
            if let Err(error) = result {
                tracing::warn!(
                    audit_log = %oid,
                    generation = snapshot.generation,
                    %error,
                    "Failed to commit Audit Log snapshot"
                );
            }
        },
    )
}

impl AuditLogObject {
    /// Commit `snapshot` and take it, waiting for the commit in place.
    pub(super) fn commit_and_apply(&mut self, snapshot: AuditLogSnapshot) -> Result<(), Error> {
        validate_snapshot(&snapshot)?;
        let snapshot = Arc::new(snapshot);
        self.writer.submit(Arc::clone(&snapshot)).take_outcome()?;
        self.apply_snapshot(snapshot);
        Ok(())
    }

    fn apply_snapshot(&mut self, snapshot: Arc<AuditLogSnapshot>) {
        let snapshot = Arc::try_unwrap(snapshot).unwrap_or_else(|shared| (*shared).clone());
        self.generation = snapshot.generation;
        self.buffer_size = snapshot.capacity;
        self.log_enable = snapshot.log_enable;
        self.total_record_count = snapshot.total_record_count;
        self.buffer = snapshot.records.into();
        self.completed_receipts = snapshot.completed_receipts;
    }

    /// Before the log builds on its state in place, let a staged commit
    /// land, waiting for it if it is still running.
    pub(super) fn settle_staged(&mut self) {
        let Some(commit) = self.staged.take() else {
            return;
        };
        commit.released.set();
        match commit.kind {
            StagedKind::Batch {
                token,
                outcome,
                changed,
            } => {
                let result = commit.ticket.take_outcome().map(|()| {
                    self.apply_snapshot(commit.snapshot);
                    (outcome, changed)
                });
                if self.settled.len() == MAX_SETTLED {
                    self.settled.pop_front();
                }
                self.settled.push_back(Settled { token, result });
            }
            StagedKind::Changes { .. } => self.drop_staged_write(&commit.ticket),
        }
    }

    /// Staged changes are dropped before the request took them all. Their
    /// commit may have left storage with a snapshot the log never served;
    /// commit the served state at the next generation so storage follows the
    /// log again. A commit made in place right after lands later and
    /// replaces it.
    fn drop_staged_write(&mut self, ticket: &SaveTicket) {
        if let Some(served) = self.served_to_put_back(ticket) {
            self.writer.submit_coalescing(Arc::new(served));
        }
    }

    /// The served state, at the next generation, that storage goes back to
    /// after staged changes committed on `ticket` are dropped; `None` when
    /// their commit failed, so storage never left the served state, or the
    /// generations have run out.
    fn served_to_put_back(&self, ticket: &SaveTicket) -> Option<AuditLogSnapshot> {
        if ticket.succeeded() == Some(false) {
            return None;
        }
        let generation = self.generation.checked_add(1)?;
        let mut served = self.current_snapshot();
        served.generation = generation;
        Some(served)
    }

    /// The log is going away (#1363). Staged changes no request finished
    /// are dropped as [`settle_staged`](Self::settle_staged) drops them, but
    /// the commit of the served state is waited for here, so it lands before
    /// the writer's own drop joins its thread, and a failure is logged. A
    /// staged batch stays as it is: storage holds it, or will once its
    /// commit runs, and the log takes a batch whose commit succeeded
    /// whenever it settles one, so a restart serving it is what this log
    /// would have done.
    fn close_staged(&mut self) {
        let Some(commit) = self
            .staged
            .take_if(|commit| matches!(commit.kind, StagedKind::Changes { .. }))
        else {
            return;
        };
        commit.released.set();
        if let Some(served) = self.served_to_put_back(&commit.ticket) {
            self.writer.put_back(Arc::new(served));
        }
    }

    /// The wait for a staged commit that still holds the log: a batch's
    /// while its commit runs, a write's until its request takes or drops it
    /// (or [`STAGED_WRITE_LIFETIME`](crate::durable::STAGED_WRITE_LIFETIME)
    /// after its commit finished). A batch whose commit ran is settled, and a
    /// forgotten write dropped.
    fn make_way(&mut self) -> Option<SaveWait> {
        let commit = self.staged.as_ref()?;
        let holds = match commit.kind {
            StagedKind::Batch { .. } => !commit.ticket.is_done(),
            StagedKind::Changes { .. } => !commit.ticket.outlived(),
        };
        if holds {
            return Some(match commit.kind {
                StagedKind::Batch { .. } => commit.ticket.wait(),
                StagedKind::Changes { .. } => SaveWait::new(Arc::clone(&commit.released)),
            });
        }
        self.settle_staged();
        None
    }

    fn queue_staged(&mut self, snapshot: Arc<AuditLogSnapshot>, kind: StagedKind) -> SaveWait {
        let ticket = self.writer.submit(Arc::clone(&snapshot));
        let saved = ticket.wait();
        self.staged = Some(StagedCommit {
            snapshot,
            ticket,
            kind,
            released: Arc::default(),
        });
        saved
    }

    /// Stage a notification batch: build the next snapshot as
    /// `store_confirmed_notifications_with_change` (with a receipt) or
    /// `store_notifications_with_change` (without) would, and queue its
    /// commit.
    pub(super) fn stage_notification_batch(
        &mut self,
        notifications: &[BACnetAuditNotification],
        apdu_timeout_ms: u32,
        receipt: Option<CompletedAuditReceipt>,
    ) -> Result<AuditBatchStage, Error> {
        if let Some(wait) = self.make_way() {
            return Ok(AuditBatchStage::Busy(wait));
        }
        if !self.log_enable {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES,
                code: ErrorCode::SERVICE_REQUEST_DENIED,
            });
        }
        if notifications.is_empty() {
            return Err(Error::OutOfRange(
                "Audit notification batch must not be empty".into(),
            ));
        }
        if let Some(receipt) = &receipt {
            if receipt::contains(
                &self.completed_receipts,
                receipt.key(),
                receipt.completed_at_unix_millis(),
            )? {
                return Ok(AuditBatchStage::Done(
                    ConfirmedAuditNotificationOutcome::Duplicate,
                    false,
                ));
            }
        }
        let mut prospective = self.snapshot_for_next_generation()?;
        let changed =
            self.apply_notification_batch(&mut prospective, notifications, apdu_timeout_ms)?;
        let confirmed = receipt.is_some();
        let outcome = match receipt {
            Some(receipt) => receipt::insert(&mut prospective.completed_receipts, receipt)?,
            None => ConfirmedAuditNotificationOutcome::Stored,
        };
        // As in place: a duplicate commits nothing, and neither does an
        // unconfirmed batch that changes no record.
        if outcome == ConfirmedAuditNotificationOutcome::Duplicate || (!confirmed && !changed) {
            return Ok(AuditBatchStage::Done(outcome, false));
        }
        let changed = changed && self.buffer_size != 0;
        validate_snapshot(&prospective)?;
        self.next_token = self.next_token.wrapping_add(1);
        let token = self.next_token;
        let saved = self.queue_staged(
            Arc::new(prospective),
            StagedKind::Batch {
                token,
                outcome,
                changed,
            },
        );
        Ok(AuditBatchStage::Staged(StagedAuditBatch { token, saved }))
    }

    /// Take a staged batch once its commit has run: apply it if the commit
    /// succeeded, or return the commit's error and leave the log as it was.
    pub(super) fn finish_notification_batch(
        &mut self,
        staged: StagedAuditBatch,
    ) -> Result<(ConfirmedAuditNotificationOutcome, bool), Error> {
        let ours = self.staged.as_ref().is_some_and(|commit| {
            matches!(commit.kind, StagedKind::Batch { token, .. } if token == staged.token)
        });
        if ours {
            self.settle_staged();
        }
        let index = self
            .settled
            .iter()
            .position(|settled| settled.token == staged.token)
            .ok_or_else(operational_problem)?;
        self.settled
            .remove(index)
            .expect("index found above")
            .result
    }

    /// Make `change`: take its step if it is the next one staged on the
    /// current state, or build it and commit it in place. Errors building it
    /// (no clock) pass through; a commit that fails is refused with DEVICE /
    /// OPERATIONAL_PROBLEM, as a Notification Forwarder refuses a list it
    /// cannot save, and leaves the log as it was. The writer has logged the
    /// storage error.
    pub(super) fn commit_change(&mut self, change: StagedChange) -> Result<(), Error> {
        if let Some(taken) = self.take_step(change) {
            return taken.map_err(|_| operational_problem());
        }
        let timestamp = match change {
            StagedChange::BufferSize(_) => None,
            StagedChange::LogEnable(_) | StagedChange::Purge => Some(self.valid_timestamp()?),
        };
        let mut prospective = self.snapshot_for_next_generation()?;
        apply_change(&mut prospective, change, timestamp)?;
        self.commit_and_apply(prospective)
            .map_err(|_| operational_problem())
    }

    /// Take the next staged step if it is `change` and builds on the state
    /// served: the first step on the state the changes were staged on, a
    /// later one on the step before it. A commit that failed drops every
    /// step and returns its error.
    fn take_step(&mut self, change: StagedChange) -> Option<Result<(), Error>> {
        let served = self.generation;
        let commit = self.staged.as_mut()?;
        let generation = commit.snapshot.generation;
        let StagedKind::Changes { steps, taken } = &mut commit.kind else {
            return None;
        };
        let base = if *taken == 0 {
            generation.checked_sub(1)
        } else {
            Some(generation)
        };
        if steps.front().map(|step| step.change) != Some(change) || base != Some(served) {
            return None;
        }
        if let Err(error) = commit.ticket.take_outcome() {
            self.staged.take().expect("matched above").released.set();
            return Some(Err(error));
        }
        let step = steps.pop_front().expect("matched above");
        *taken += 1;
        if steps.is_empty() {
            self.staged.take().expect("matched above").released.set();
        }
        self.apply_snapshot(step.state);
        Some(Ok(()))
    }

    /// From the operation task: settle a batch whose commit has run, and drop
    /// a write its request forgot.
    pub(super) fn settle_finished(&mut self) -> bool {
        let Some(commit) = &self.staged else {
            return false;
        };
        let finished = commit.ticket.is_done()
            && match commit.kind {
                StagedKind::Batch { .. } => true,
                StagedKind::Changes { .. } => commit.ticket.outlived(),
            };
        if finished {
            self.settle_staged();
        }
        finished
    }

    /// Block until every queued commit has run.
    #[cfg(test)]
    pub(super) fn wait_for_commits(&self) {
        self.writer.wait_idle();
    }

    /// Build a step for each of `changes` in turn and queue one commit of
    /// the last. A change that cannot be built here, such as a purge without
    /// a valid clock, ends the steps: made under the guard, it then fails as
    /// it would unstaged, and the request makes no later change.
    fn stage_changes(&mut self, changes: &[StagedChange]) -> StageStep {
        let Ok(mut state) = self.snapshot_for_next_generation() else {
            return StageStep::Skip;
        };
        let timestamp: Option<Timestamp> = self.valid_timestamp().ok();
        let mut steps = VecDeque::with_capacity(changes.len());
        // The change made to `state` but not yet kept as a step. Only the
        // states between changes are copied; the last one is moved.
        let mut made: Option<StagedChange> = None;
        for &change in changes {
            let before = made.map(|change| (change, state.clone()));
            if apply_change(&mut state, change, timestamp).is_err() {
                break;
            }
            if let Some((change, kept)) = before {
                steps.push_back(Step {
                    change,
                    state: Arc::new(kept),
                });
            }
            made = Some(change);
        }
        let Some(last) = made else {
            return StageStep::Skip;
        };
        if validate_snapshot(&state).is_err() {
            return StageStep::Skip;
        }
        let snapshot = Arc::new(state);
        steps.push_back(Step {
            change: last,
            state: Arc::clone(&snapshot),
        });
        StageStep::Staged(self.queue_staged(snapshot, StagedKind::Changes { steps, taken: 0 }))
    }
}

/// The Log_Enable and Buffer_Size changes `writes` make, in order, each
/// judged as the log will be when the request makes it. A write the log will
/// refuse ends the list, since the request makes no write after it; a write
/// of the value the log will already hold changes nothing; other properties
/// commit nothing. Neither property takes a NULL, so one the log refuses as
/// the wrong datatype changes nothing either: the server answers it as a
/// relinquish that leaves the property as it is, and the request goes on
/// (#1396).
fn changes_made(
    mut log_enable: bool,
    mut buffer_size: u32,
    writes: &[PendingWrite],
) -> Vec<StagedChange> {
    let mut changes = Vec::new();
    for write in writes {
        let change = match (write.property, write.array_index, &write.value) {
            (PropertyIdentifier::LOG_ENABLE, None, PropertyValue::Boolean(on)) => {
                if *on == log_enable {
                    continue;
                }
                log_enable = *on;
                StagedChange::LogEnable(*on)
            }
            (PropertyIdentifier::LOG_ENABLE, None, PropertyValue::Null) => continue,
            (PropertyIdentifier::BUFFER_SIZE, None, value) => {
                match written_buffer_size(log_enable, value) {
                    Ok(size) if size == buffer_size => continue,
                    Ok(size) => {
                        buffer_size = size;
                        StagedChange::BufferSize(size)
                    }
                    // Logging off, a NULL is refused only as a datatype.
                    Err(_) if *value == PropertyValue::Null && !log_enable => continue,
                    Err(_) => break,
                }
            }
            (PropertyIdentifier::LOG_ENABLE | PropertyIdentifier::BUFFER_SIZE, ..) => break,
            _ => continue,
        };
        changes.push(change);
    }
    changes
}

impl DurableWrites for AuditLogObject {
    fn stage_write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep {
        self.stage_writes(&[PendingWrite {
            property,
            array_index,
            value: value.clone(),
        }])
    }

    /// Fold the request's Log_Enable and Buffer_Size writes into one staged
    /// commit, so a WritePropertyMultiple that turns logging off and then
    /// resizes commits both off the guard.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        if !writes.iter().any(|write| {
            matches!(
                write.property,
                PropertyIdentifier::LOG_ENABLE | PropertyIdentifier::BUFFER_SIZE
            )
        }) {
            return StageStep::Skip;
        }
        // Wait for a change staged ahead of these first: they are judged by
        // the log as it is once that lands.
        if let Some(wait) = self.make_way() {
            return StageStep::Busy(wait);
        }
        let changes = changes_made(self.log_enable, self.buffer_size, writes);
        self.stage_changes(&changes)
    }

    fn stage_purge(&mut self) -> StageStep {
        if let Some(wait) = self.make_way() {
            return StageStep::Busy(wait);
        }
        self.stage_changes(&[StagedChange::Purge])
    }

    fn commit_purge(&mut self) -> Result<(), Error> {
        self.purge().map(drop)
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        if self.staged.as_ref().is_some_and(|commit| {
            matches!(commit.kind, StagedKind::Changes { .. }) && commit.ticket.issued(staged)
        }) {
            self.settle_staged();
        }
    }

    fn has_staged_write(&self) -> bool {
        self.staged.is_some()
    }

    /// Settle what the operation task would, without waiting out a
    /// lifetime: drop staged changes, putting storage back, and take a
    /// batch whose commit has run. A batch whose commit still runs is left
    /// to land; the wait returned covers it.
    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        if self.staged.as_ref().is_some_and(|commit| {
            commit.ticket.is_done() || matches!(commit.kind, StagedKind::Changes { .. })
        }) {
            self.settle_staged();
        }
        Some(self.writer.idle())
    }
}

impl Drop for AuditLogObject {
    fn drop(&mut self) {
        self.close_staged();
    }
}
