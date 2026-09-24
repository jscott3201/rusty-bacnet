//! Shared database-local timestamp ordering authority.
use std::sync::Mutex;

/// One database's clockless timestamp source, also borrowed by its Device sink.
#[doc(hidden)]
#[derive(Default)]
pub struct EventSequence(Mutex<u16>);

impl EventSequence {
    /// Serialize preparation and commit without consuming on an error. The
    /// closure must not reenter this authority or invoke arbitrary user code.
    pub fn transaction<R, E>(
        &self,
        prepare_commit: impl FnOnce(u16) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut number = self.0.lock().unwrap();
        let value = prepare_commit(*number)?;
        *number = number.wrapping_add(1);
        Ok(value)
    }
    /// Reserve a bounded aggregate's consecutive timestamps, committing none on error.
    pub fn transaction_many<R, E>(
        &self,
        count: u16,
        prepare_commit: impl FnOnce(u16) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut number = self.0.lock().unwrap();
        let result = prepare_commit(*number)?;
        *number = number.wrapping_add(count);
        Ok(result)
    }
    pub(super) fn current(&self) -> u16 {
        *self.0.lock().unwrap()
    }
    pub(super) fn confirm(&self, expected: u16) -> bool {
        let mut number = self.0.lock().unwrap();
        if *number != expected {
            return false;
        }
        *number = number.wrapping_add(1);
        true
    }
}
