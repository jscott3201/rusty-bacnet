//! The runs a Present_Value write queues on a Command or Channel object
//! (Clauses 12.10 and 12.53): taking them under the guard that committed the
//! write, and ending one where it stood, or at once, unmade, for a host that
//! can't make its writes.
use alloc::vec::Vec;
use bacnet_objects::command::{CommandRun, RunPlan, WriteFailure};
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::primitives::ObjectIdentifier;

/// Where a run stood when its future let go of it.
#[derive(Debug, Clone, Copy)]
pub struct Unfinished {
    source: ObjectIdentifier,
    generation: u64,
    /// How many writes have been made: a Command's first `next` commands, in
    /// list order, or that many of a Channel's members, in the order they
    /// finished.
    next: usize,
    len: usize,
    /// `Ok` while every write made so far succeeded, otherwise the first
    /// failure.
    outcome: Result<(), WriteFailure>,
}

impl Unfinished {
    /// `run` before any of its writes is made.
    pub fn start(run: &CommandRun) -> Self {
        Self {
            source: run.source,
            generation: run.generation,
            next: 0,
            len: match &run.plan {
                RunPlan::Actions(commands) => commands.len(),
                RunPlan::Channel(distribution) => distribution.members.len(),
            },
            outcome: Ok(()),
        }
    }

    /// The object the run belongs to.
    pub fn source(&self) -> ObjectIdentifier {
        self.source
    }

    /// `next` writes have been made; `outcome` is the first failure among
    /// them, if any.
    pub fn progress(&mut self, next: usize, outcome: Result<(), WriteFailure>) {
        self.next = next;
        self.outcome = outcome;
    }

    /// Whether this is `source`'s run of `generation`.
    pub fn is(&self, source: ObjectIdentifier, generation: u64) -> bool {
        self.source == source && self.generation == generation
    }

    /// End the run where it stood, as successful only if every write was
    /// made and succeeded. A Command's commands never made read unsuccessful
    /// and In_Process returns to FALSE; a Channel's Write_Status becomes
    /// SUCCESSFUL or FAILED. A run cut short before a write failed fails as a
    /// process error. Nothing changes once the object has moved on to another
    /// generation. Returns whether the run ended here.
    pub fn end(self, db: &mut ObjectDatabase) -> bool {
        let outcome = if self.next < self.len {
            self.outcome.and(Err(WriteFailure::Process))
        } else {
            self.outcome
        };
        db.get_mut(&self.source).is_some_and(|object| {
            for index in self.next..self.len {
                object.record_command_write_internal(self.generation, index, false);
            }
            object.complete_command_run_internal(self.generation, outcome)
        })
    }
}

/// Take the runs that Present_Value writes queued on Command and Channel
/// objects among `oids`, under the guard that committed those writes. The
/// caller owns them; the server's `TakenRuns` holds them so a drop ends them.
pub fn take_queued(db: &mut ObjectDatabase, oids: &[ObjectIdentifier]) -> Vec<CommandRun> {
    oids.iter()
        .filter_map(|oid| {
            db.get_mut(oid)
                .and_then(|object| object.take_command_run_internal())
        })
        .collect()
}

/// End the runs that Present_Value writes queued on Command and Channel
/// objects among `oids` without making any of their writes, for an owner
/// that can't run them. Each ends at once as unsuccessful, so none is left
/// busy.
pub fn end_unmade(db: &mut ObjectDatabase, oids: &[ObjectIdentifier]) {
    for run in take_queued(db, oids) {
        Unfinished::start(&run).end(db);
    }
}
