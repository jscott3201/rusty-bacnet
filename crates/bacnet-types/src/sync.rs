//! Taking std's locks whether a panic poisoned them or not (#2559).
//!
//! A [`Mutex`] or [`RwLock`] is poisoned when a thread panics while holding
//! it, and from then on its lock calls return an error. Unwrapping that
//! error turns the one panic into a panic in every later user of the lock,
//! so a panic in one request's task could take a whole server, link or
//! router down with it. The stack never panics on a poisoned lock: by
//! default it takes the lock through these extension traits and leaves the
//! panic to the task it happened in, with the data as the panicking holder
//! left it. Where a half-done update would leave data that must not be
//! used, the code returns an error on poison instead (fails closed), and
//! says why where it locks.
//!
//! Each method is the std method it is named after, with the poison error
//! unwrapped to its guard or reference: no other cost. The traits are
//! sealed: only std's types implement them, so methods can be added later.
//! In the rusty-bacnet repository, a CI script
//! (`scripts/ci/check-lock-poison.py`) holds the stack's own code to this
//! rule.
//!
//! ```
//! use bacnet_types::sync::MutexExt;
//! use std::sync::Mutex;
//!
//! let count = Mutex::new(0);
//! *count.locked() += 1;
//! assert_eq!(*count.locked(), 1);
//! ```

use std::sync::{
    Condvar, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
    TryLockError, WaitTimeoutResult,
};
use std::time::Duration;

/// Only std's lock types implement the traits here, so adding a method to
/// one is not a breaking change.
mod sealed {
    pub trait Sealed {}
    impl<T: ?Sized> Sealed for std::sync::Mutex<T> {}
    impl<T: ?Sized> Sealed for std::sync::RwLock<T> {}
    impl Sealed for std::sync::Condvar {}
}

/// [`Mutex`] access that doesn't fail when the mutex is poisoned.
pub trait MutexExt<T: ?Sized>: sealed::Sealed {
    /// [`Mutex::lock`], poisoned or not.
    fn locked(&self) -> MutexGuard<'_, T>;
    /// [`Mutex::try_lock`], poisoned or not: `None` only while another
    /// thread holds the lock.
    fn try_locked(&self) -> Option<MutexGuard<'_, T>>;
    /// [`Mutex::get_mut`], poisoned or not.
    fn get_mut_locked(&mut self) -> &mut T;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    #[inline]
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[inline]
    fn try_locked(&self) -> Option<MutexGuard<'_, T>> {
        match self.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    #[inline]
    fn get_mut_locked(&mut self) -> &mut T {
        self.get_mut().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`RwLock`] access that doesn't fail when the lock is poisoned.
pub trait RwLockExt<T: ?Sized>: sealed::Sealed {
    /// [`RwLock::read`], poisoned or not.
    fn read_locked(&self) -> RwLockReadGuard<'_, T>;
    /// [`RwLock::write`], poisoned or not.
    fn write_locked(&self) -> RwLockWriteGuard<'_, T>;
    /// [`RwLock::get_mut`], poisoned or not.
    fn get_mut_locked(&mut self) -> &mut T;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    #[inline]
    fn read_locked(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    #[inline]
    fn write_locked(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }

    #[inline]
    fn get_mut_locked(&mut self) -> &mut T {
        self.get_mut().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`Condvar`] waits that take the mutex back whether it was poisoned or not.
pub trait CondvarExt: sealed::Sealed {
    /// [`Condvar::wait`], poisoned or not.
    fn wait_locked<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T>;
    /// [`Condvar::wait_timeout_while`], poisoned or not.
    fn wait_timeout_while_locked<'a, T, F>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
        condition: F,
    ) -> (MutexGuard<'a, T>, WaitTimeoutResult)
    where
        F: FnMut(&mut T) -> bool;
}

impl CondvarExt for Condvar {
    #[inline]
    fn wait_locked<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.wait(guard).unwrap_or_else(PoisonError::into_inner)
    }

    #[inline]
    fn wait_timeout_while_locked<'a, T, F>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
        condition: F,
    ) -> (MutexGuard<'a, T>, WaitTimeoutResult)
    where
        F: FnMut(&mut T) -> bool,
    {
        self.wait_timeout_while(guard, timeout, condition)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::thread;

    /// Run `f`, which panics: the panic poisons whatever lock it holds.
    fn panic_in(f: impl FnOnce()) {
        assert!(catch_unwind(AssertUnwindSafe(f)).is_err());
    }

    #[test]
    fn a_poisoned_mutex_is_taken_as_the_panic_left_it() {
        let mut mutex = Mutex::new(vec![1]);
        panic_in(|| {
            let mut guard = mutex.lock().unwrap();
            guard.push(2);
            panic!("poison the mutex mid-update");
        });
        assert!(mutex.is_poisoned());
        assert_eq!(*mutex.locked(), [1, 2]);
        mutex.locked().push(3);
        mutex.get_mut_locked().push(4);
        assert_eq!(*mutex.locked(), [1, 2, 3, 4]);
        assert!(
            mutex.is_poisoned(),
            "taking the lock doesn't clear the poison"
        );
    }

    #[test]
    fn a_try_lock_takes_a_free_lock_poisoned_or_not_and_never_a_held_one() {
        let mutex = Mutex::new(1);
        assert_eq!(mutex.try_locked().as_deref(), Some(&1));
        let held = mutex.locked();
        assert!(mutex.try_locked().is_none());
        drop(held);
        panic_in(|| {
            let mut guard = mutex.lock().unwrap();
            *guard = 2;
            panic!("poison the mutex mid-update");
        });
        assert!(mutex.is_poisoned());
        assert_eq!(mutex.try_locked().as_deref(), Some(&2));
        let held = mutex.locked();
        assert!(
            mutex.try_locked().is_none(),
            "held and poisoned is still held"
        );
        drop(held);
    }

    #[test]
    fn a_poisoned_rwlock_is_taken_for_reading_and_writing() {
        let mut lock = RwLock::new(String::from("before"));
        panic_in(|| {
            let mut guard = lock.write().unwrap();
            guard.push_str(", during");
            panic!("poison the lock mid-update");
        });
        assert!(lock.is_poisoned());
        assert_eq!(*lock.read_locked(), "before, during");
        lock.write_locked().push_str(", after");
        lock.get_mut_locked().push('.');
        assert_eq!(*lock.read_locked(), "before, during, after.");
    }

    #[test]
    fn a_condvar_wait_takes_a_poisoned_mutex_back() {
        let shared = Arc::new((Mutex::new(false), Condvar::new()));
        panic_in(|| {
            let _guard = shared.0.lock().unwrap();
            panic!("poison the mutex");
        });
        assert!(shared.0.is_poisoned());
        // Held before the waker starts, so this thread waits at least once:
        // the waker can't set the flag until the wait lets the mutex go.
        let mut ready = shared.0.locked();
        let waker = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                *shared.0.locked() = true;
                shared.1.notify_all();
            })
        };
        while !*ready {
            ready = shared.1.wait_locked(ready);
        }
        drop(ready);
        waker.join().unwrap();
        // A condition that stays true, so the wait runs (std returns before
        // it looks at the poison when the condition is already false), and
        // nothing notifies: it times out and takes the poisoned mutex back.
        let (ready, timeout) = shared.1.wait_timeout_while_locked(
            shared.0.locked(),
            Duration::from_millis(1),
            |ready| *ready,
        );
        assert!(*ready);
        assert!(timeout.timed_out());
    }
}
