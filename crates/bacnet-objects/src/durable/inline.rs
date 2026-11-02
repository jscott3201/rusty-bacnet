//! The save writer without std: each save runs on the caller's thread
//! (#1615).
//!
//! There are no threads to hand a save to, so a save runs where it is
//! queued, before the call that queued it returns. Every [`SaveTicket`] is
//! complete by then, so the objects' staging still works as the [module
//! documentation](super) describes, only with each wait already over: an
//! object holding the database guard makes the storage call itself. A
//! storage call that panics is not caught, since only std can catch one.

use alloc::sync::Arc;

use bacnet_types::error::Error;

use super::{Event, SaveTicket, SaveWait};
use crate::prelude::*;

type SaveFn<S> = dyn Fn(&S) -> Result<(), Error> + Send + Sync;
type DoneFn<S> = dyn Fn(&S, &Result<(), Error>) + Send + Sync;

/// Runs one object's storage calls as they are queued, on the caller's
/// thread. It has the hosted writer's interface, so the objects are the same
/// in both builds.
pub(crate) struct SaveWriter<S: Send + 'static> {
    save: Box<SaveFn<S>>,
    done: Box<DoneFn<S>>,
}

impl<S: Send + 'static> SaveWriter<S> {
    /// A writer that saves each snapshot with `save` and then reports the
    /// result to `done`. The hosted writer names its thread `name`; this one
    /// has no thread to name.
    pub(crate) fn new(
        name: String,
        save: impl Fn(&S) -> Result<(), Error> + Send + Sync + 'static,
        done: impl Fn(&S, &Result<(), Error>) + Send + Sync + 'static,
    ) -> Self {
        drop(name);
        Self {
            save: Box::new(save),
            done: Box::new(done),
        }
    }

    /// Save now, and hand back the finished save's ticket.
    pub(crate) fn submit(&mut self, snapshot: S) -> SaveTicket {
        let ticket = SaveTicket::default();
        self.run(snapshot, Some(&ticket));
        ticket
    }

    /// Save now. Nothing waits, so nothing is queued to replace.
    pub(crate) fn submit_coalescing(&mut self, snapshot: S) {
        self.run(snapshot, None);
    }

    /// A wait that is already over: every save has run.
    pub(crate) fn idle(&self) -> SaveWait {
        let event = Arc::new(Event::default());
        event.set();
        SaveWait::new(event)
    }

    /// Every save has already run.
    pub(crate) fn wait_idle(&self) {}

    /// Save `served`, the state the object serves, as it goes away. A save
    /// that fails is reported to the done hook; storage then keeps the
    /// staged state, and the next start serves it.
    pub(crate) fn put_back(&mut self, served: S) {
        let _ = self.submit(served).take_outcome();
    }

    /// Make one storage call, report it to the done hook, drop the snapshot
    /// and complete `ticket`, in that order, as the hosted writer does.
    fn run(&self, snapshot: S, ticket: Option<&SaveTicket>) {
        let result = (self.save)(&snapshot);
        (self.done)(&snapshot, &result);
        drop(snapshot);
        if let Some(ticket) = ticket {
            ticket.complete(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use std::sync::Mutex;

    use bacnet_types::error::Error;

    use super::SaveWriter;

    /// A writer that records each save and done call, and refuses `13`.
    fn writer(log: &Arc<Mutex<Vec<String>>>) -> SaveWriter<u32> {
        let (saves, dones) = (Arc::clone(log), Arc::clone(log));
        SaveWriter::new(
            "test".into(),
            move |n: &u32| {
                saves.lock().unwrap().push(format!("save {n}"));
                if *n == 13 {
                    Err(Error::Encoding("refused".into()))
                } else {
                    Ok(())
                }
            },
            move |n, result| {
                dones
                    .lock()
                    .unwrap()
                    .push(format!("done {n} {}", result.is_ok()));
            },
        )
    }

    /// Blocking on a wait nothing on this thread can end panics in a debug
    /// build instead of spinning forever.
    #[test]
    #[should_panic(expected = "await it instead")]
    fn blocking_on_a_wait_that_isnt_over_panics() {
        let wait = super::SaveWait::new(Arc::new(super::Event::default()));
        wait.block();
    }

    #[test]
    fn each_save_runs_before_the_call_that_queued_it_returns() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut writer = writer(&log);

        let ticket = writer.submit(1);
        assert!(ticket.is_done() && ticket.wait().is_ready());
        assert_eq!(ticket.succeeded(), Some(true));
        writer.submit_coalescing(2);
        writer.submit_coalescing(3);
        let failed = writer.submit(13);
        assert!(failed.take_outcome().is_err());
        assert!(writer.idle().is_ready());
        writer.wait_idle();
        writer.put_back(4);

        assert_eq!(
            *log.lock().unwrap(),
            [
                "save 1",
                "done 1 true",
                "save 2",
                "done 2 true",
                "save 3",
                "done 3 true",
                "save 13",
                "done 13 false",
                "save 4",
                "done 4 true",
            ]
        );
    }
}
