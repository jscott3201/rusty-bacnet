//! Linux thread scheduling: `pthread_setschedparam` and its read-back, for
//! the MS/TP transport's dedicated thread (#501). nix wraps
//! `sched_setaffinity` but not the scheduling policy, so the policy calls
//! are here.
//!
//! The pthread calls act on one thread, the caller's (`pthread_self`), by
//! definition, and work on glibc and musl alike; musl's
//! `sched_setscheduler` only returns `ENOSYS`. Both return their error
//! number rather than setting `errno`.

/// A `sched_param` holding `priority` and zeros elsewhere (musl's has
/// fields beyond the priority).
fn param(priority: libc::c_int) -> libc::sched_param {
    // SAFETY: `sched_param` is a plain C struct of integers, for which all
    // zeros is a valid value.
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    param.sched_priority = priority;
    param
}

/// `Ok` for a pthread call's zero, the error number it returned otherwise.
fn result(ret: libc::c_int) -> std::io::Result<()> {
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(ret))
    }
}

/// Put the calling thread under `SCHED_FIFO` at `priority`.
///
/// # Errors
///
/// The OS error: `EPERM` without `CAP_SYS_NICE` or an `RLIMIT_RTPRIO` of
/// at least `priority` (or without real-time budget in the thread's
/// cgroup), `EINVAL` for a priority outside 1..=99.
pub(crate) fn set_fifo(priority: libc::c_int) -> std::io::Result<()> {
    let param = param(priority);
    // SAFETY: `pthread_self` is always a valid handle to the calling
    // thread, which is alive for the call; `param` is a valid, initialized
    // `sched_param` borrowed for the call, which only reads it.
    result(unsafe { libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param) })
}

/// Whether the calling thread runs under `SCHED_FIFO`, and its static
/// priority.
///
/// # Errors
///
/// The OS error of the read.
pub(crate) fn current() -> std::io::Result<(bool, libc::c_int)> {
    let mut policy: libc::c_int = 0;
    let mut param = param(0);
    // SAFETY: `pthread_self` is always a valid handle to the calling
    // thread, which is alive for the call; `policy` and `param` are valid,
    // writable locals borrowed for the call, which writes only them.
    result(unsafe { libc::pthread_getschedparam(libc::pthread_self(), &mut policy, &mut param) })?;
    Ok((policy == libc::SCHED_FIFO, param.sched_priority))
}
