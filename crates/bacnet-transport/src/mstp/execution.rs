//! Where the MS/TP MAC loop runs: the caller's Tokio runtime, or a thread of
//! its own, which on Linux can run under `SCHED_FIFO` and be pinned to CPUs
//! (#501).

use bacnet_types::error::Error;
use tokio::sync::oneshot;

/// Execution placement for an MS/TP transport, independent of master-node settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MstpExecutionMode {
    /// Spawn the MAC loop on the caller's Tokio runtime (the default).
    #[default]
    Tokio,
    /// Poll the same MAC loop on a dedicated OS thread with its own Tokio
    /// runtime, set up as the [`MstpThreadConfig`] says. Blocking drain work
    /// uses that runtime's blocking pool, not the application's.
    ///
    /// The thread alone isolates the loop from a busy application runtime,
    /// not from the OS scheduler. On Linux the config can also ask for
    /// real-time scheduling and CPU affinity, which lower the wake-up
    /// latency the scheduler adds. Neither is a timing guarantee.
    DedicatedThread(MstpThreadConfig),
}

impl MstpExecutionMode {
    /// [`Self::DedicatedThread`] with no scheduling settings: the thread
    /// runs under the OS's normal policy, on any CPU.
    pub const fn dedicated_thread() -> Self {
        Self::DedicatedThread(MstpThreadConfig::new())
    }

    /// Check the setting as [`start`](crate::port::TransportPort::start)
    /// does; see [`MstpThreadConfig::validate`].
    ///
    /// # Errors
    ///
    /// The errors of [`MstpThreadConfig::validate`].
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Tokio => Ok(()),
            Self::DedicatedThread(config) => config.validate(),
        }
    }
}

/// The lowest `SCHED_FIFO` priority Linux takes.
pub const MIN_REALTIME_PRIORITY: u8 = 1;

/// The highest `SCHED_FIFO` priority Linux takes. Threaded interrupt
/// handlers run at `SCHED_FIFO` 50 by default, so a loop that must not hold
/// off its own UART's interrupt thread belongs below that, or the UART's
/// thread above it.
pub const MAX_REALTIME_PRIORITY: u8 = 99;

/// The most CPUs an affinity set can name: glibc's `CPU_SETSIZE`, the size
/// of nix's `CpuSet`. CPU numbers run from 0 to one less. On a machine with
/// more CPUs the thread can only be pinned among the first 1024, and the
/// read-back covers only those.
pub const MAX_CPUS: usize = 1024;

/// How the dedicated MS/TP thread is scheduled
/// ([`MstpExecutionMode::DedicatedThread`]). Everything is off by default,
/// and both settings are Linux only: elsewhere
/// [`start`](crate::port::TransportPort::start) refuses them rather than
/// run without them.
///
/// The thread applies them to itself before it builds its runtime or touches
/// the serial port, and reads each back. If one can't be applied, or the
/// kernel applied something else, `start` fails with an error naming it and
/// keeps the serial port, so the caller can fix the host or the config and
/// start again; there is no fallback to running without. What was applied
/// is in [`MstpExecutionStatus`].
///
/// Threads the loop's runtime starts later, for blocking serial calls such
/// as the drain, start from this thread, and new Linux threads inherit
/// their creator's policy, priority and affinity.
///
/// Build one from [`Self::new`] (or `Default`) and the `with_` methods.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpThreadConfig {
    /// Run the thread under `SCHED_FIFO` at this priority,
    /// [`MIN_REALTIME_PRIORITY`] to [`MAX_REALTIME_PRIORITY`]; `None` keeps
    /// the normal policy.
    ///
    /// `SCHED_FIFO` rather than `SCHED_RR`: the two differ only in sharing
    /// the CPU among threads of the same priority, by time slice. The loop
    /// spends nearly all its time blocked on a read or a timer, and one
    /// that held its CPU long enough for a slice to matter would be failing
    /// MS/TP timing already.
    ///
    /// The process needs `CAP_SYS_NICE`, or an `RLIMIT_RTPRIO` of at least
    /// this priority (`ulimit -r`, or `LimitRTPRIO=` in a systemd unit),
    /// and its cgroup a real-time budget where the kernel has real-time
    /// group scheduling; without them, `start` fails with
    /// `PermissionDenied`.
    pub realtime_priority: Option<u8>,
    /// Pin the thread to these CPUs, numbered from 0 as Linux numbers them;
    /// empty leaves its affinity alone. Order and repeats don't matter.
    ///
    /// Any CPU the process may use can be named without privilege. A CPU
    /// outside the process's cpuset (a container's, say) makes `start`
    /// fail: refused outright when none is allowed, and when the kernel
    /// applies only part of the set, found by the read-back.
    pub cpu_affinity: Vec<usize>,
}

impl MstpThreadConfig {
    /// No settings: the normal policy, on any CPU.
    pub const fn new() -> Self {
        Self {
            realtime_priority: None,
            cpu_affinity: Vec::new(),
        }
    }

    /// Ask for `SCHED_FIFO` at `priority`; see [`Self::realtime_priority`].
    pub fn with_realtime_priority(mut self, priority: u8) -> Self {
        self.realtime_priority = Some(priority);
        self
    }

    /// Pin the thread to `cpus`; see [`Self::cpu_affinity`].
    pub fn with_cpu_affinity(mut self, cpus: impl IntoIterator<Item = usize>) -> Self {
        self.cpu_affinity = cpus.into_iter().collect();
        self
    }

    /// Check the settings as [`start`](crate::port::TransportPort::start)
    /// does, without applying them.
    ///
    /// # Errors
    ///
    /// [`Error::Encoding`] for a priority outside
    /// [`MIN_REALTIME_PRIORITY`]..=[`MAX_REALTIME_PRIORITY`] or a CPU
    /// number of [`MAX_CPUS`] or more. On any OS but Linux, either setting
    /// at all is [`Error::Transport`] with
    /// [`std::io::ErrorKind::Unsupported`].
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(priority) = self.realtime_priority {
            if !(MIN_REALTIME_PRIORITY..=MAX_REALTIME_PRIORITY).contains(&priority) {
                return Err(Error::Encoding(format!(
                    "MS/TP thread realtime_priority {priority} is outside \
                     {MIN_REALTIME_PRIORITY}..={MAX_REALTIME_PRIORITY}"
                )));
            }
        }
        if let Some(cpu) = self.cpu_affinity.iter().find(|cpu| **cpu >= MAX_CPUS) {
            return Err(Error::Encoding(format!(
                "MS/TP thread cpu_affinity names CPU {cpu}; CPUs are numbered 0 to {}",
                MAX_CPUS - 1
            )));
        }
        if !cfg!(target_os = "linux") && !self.is_default() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "MS/TP thread realtime_priority and cpu_affinity are Linux only, \
                     and this is {}",
                    std::env::consts::OS
                ),
            )));
        }
        Ok(())
    }

    fn is_default(&self) -> bool {
        self.realtime_priority.is_none() && self.cpu_affinity.is_empty()
    }

    /// The affinity set as the read-back reports it: ascending, no repeats.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn wanted_cpus(&self) -> Vec<usize> {
        let mut cpus = self.cpu_affinity.clone();
        cpus.sort_unstable();
        cpus.dedup();
        cpus
    }
}

/// How the MS/TP MAC loop runs, as set up at
/// [`start`](crate::port::TransportPort::start): from
/// [`MstpTransport::execution_status`](super::MstpTransport::execution_status)
/// or [`MstpDiagnostics::execution_status`](super::MstpDiagnostics::execution_status).
///
/// Each setting is read back from the kernel after it was applied, on the
/// thread itself. A setting that could not be applied fails `start`, so a
/// started transport never runs with less than it asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpExecutionStatus {
    /// The loop runs on a thread of its own
    /// ([`MstpExecutionMode::DedicatedThread`]).
    pub dedicated_thread: bool,
    /// The `SCHED_FIFO` priority the thread runs at; `None` when none was
    /// asked for.
    pub realtime_priority: Option<u8>,
    /// The CPUs the thread may run on, ascending; empty when no affinity
    /// was asked for.
    pub cpu_affinity: Vec<usize>,
}

/// Apply `config` to the calling thread and read it back.
#[cfg(target_os = "linux")]
pub(crate) fn apply(config: &MstpThreadConfig) -> Result<MstpExecutionStatus, Error> {
    config.validate()?;
    let mut status = MstpExecutionStatus {
        dedicated_thread: true,
        ..MstpExecutionStatus::default()
    };
    // Affinity first, so the thread never runs real-time on a CPU it
    // wasn't meant to.
    if !config.cpu_affinity.is_empty() {
        status.cpu_affinity = set_affinity(&config.wanted_cpus())?;
    }
    if let Some(priority) = config.realtime_priority {
        crate::sys::sched::set_fifo(priority.into()).map_err(|error| {
            let hint = if error.kind() == std::io::ErrorKind::PermissionDenied {
                format!(
                    "; the process needs CAP_SYS_NICE or an RLIMIT_RTPRIO of at least \
                     {priority} (or the cgroup's real-time budget)"
                )
            } else {
                String::new()
            };
            thread_error(
                &error,
                format!("SCHED_FIFO priority {priority} was refused: {error}{hint}"),
            )
        })?;
        let (fifo, applied) = crate::sys::sched::current().map_err(|error| {
            thread_error(
                &error,
                format!("reading back the scheduling policy failed: {error}"),
            )
        })?;
        if !fifo || applied != libc::c_int::from(priority) {
            return Err(mismatch(format!(
                "SCHED_FIFO priority {priority} was asked for, and the thread reads back \
                 {} at priority {applied}",
                if fifo { "SCHED_FIFO" } else { "another policy" }
            )));
        }
        status.realtime_priority = Some(priority);
    }
    if status != MstpExecutionStatus::default() {
        tracing::info!(
            realtime_priority = ?status.realtime_priority,
            cpu_affinity = ?status.cpu_affinity,
            "MS/TP thread scheduling applied"
        );
    }
    Ok(status)
}

/// Every other OS: [`MstpThreadConfig::validate`] refuses any setting, so
/// there is nothing to apply.
#[cfg(not(target_os = "linux"))]
pub(crate) fn apply(config: &MstpThreadConfig) -> Result<MstpExecutionStatus, Error> {
    config.validate()?;
    Ok(MstpExecutionStatus {
        dedicated_thread: true,
        ..MstpExecutionStatus::default()
    })
}

/// Pin the calling thread to `wanted` (ascending, no repeats) and return
/// what the kernel reads back, which must be the same.
#[cfg(target_os = "linux")]
fn set_affinity(wanted: &[usize]) -> Result<Vec<usize>, Error> {
    use nix::sched::{sched_getaffinity, sched_setaffinity, CpuSet};
    use nix::unistd::Pid;

    let failed = |what: &str, errno: nix::Error| {
        let error = std::io::Error::from(errno);
        thread_error(&error, format!("{what} failed: {error}"))
    };
    let mut set = CpuSet::new();
    for &cpu in wanted {
        set.set(cpu)
            .map_err(|errno| failed(&format!("adding CPU {cpu} to the affinity set"), errno))?;
    }
    sched_setaffinity(Pid::from_raw(0), &set).map_err(|errno| {
        failed(
            &format!("setting CPU affinity {wanted:?} (CPUs outside the process's cpuset?)"),
            errno,
        )
    })?;
    let got = sched_getaffinity(Pid::from_raw(0))
        .map_err(|errno| failed("reading back the CPU affinity", errno))?;
    let applied: Vec<usize> = (0..CpuSet::count())
        .filter(|cpu| got.is_set(*cpu).unwrap_or(false))
        .collect();
    if applied != wanted {
        return Err(mismatch(format!(
            "CPU affinity {wanted:?} was asked for, and the kernel applied {applied:?}; \
             the cpuset or the online CPUs don't allow the rest"
        )));
    }
    Ok(applied)
}

#[cfg(target_os = "linux")]
fn thread_error(error: &std::io::Error, message: String) -> Error {
    Error::Transport(std::io::Error::new(
        error.kind(),
        format!("MS/TP thread: {message}"),
    ))
}

#[cfg(target_os = "linux")]
fn mismatch(message: String) -> Error {
    Error::Transport(std::io::Error::other(format!("MS/TP thread: {message}")))
}

/// Owns the isolated reactor. Dropping the shutdown sender also handles cancelled
/// startup and abort; only `stop` waits for runtime and blocking-I/O completion.
pub(super) struct DedicatedRuntime {
    pub(super) handle: tokio::runtime::Handle,
    shutdown: oneshot::Sender<()>,
    finished: oneshot::Receiver<()>,
}

impl DedicatedRuntime {
    /// Start the thread, apply `config` to it, and build its runtime. An
    /// error from any step ends the thread and comes back here.
    pub(super) async fn new(
        config: MstpThreadConfig,
    ) -> Result<(Self, MstpExecutionStatus), Error> {
        let (started_tx, started_rx) = oneshot::channel();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let (finished_tx, finished) = oneshot::channel();
        std::thread::Builder::new()
            .name("bacnet-mstp".into())
            .spawn(move || {
                // Before the runtime exists, so every thread it starts
                // inherits the settings.
                let started = apply(&config).and_then(|status| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map(|runtime| (runtime, status))
                        .map_err(Error::Transport)
                });
                match started {
                    Ok((runtime, status)) => {
                        if started_tx
                            .send(Ok((runtime.handle().clone(), status)))
                            .is_ok()
                        {
                            let _ = runtime.block_on(shutdown_rx);
                        }
                        // Drop on the OS thread, never in the caller's async context.
                        // This waits for any already-started blocking drain calls.
                        drop(runtime);
                    }
                    Err(error) => {
                        let _ = started_tx.send(Err(error));
                    }
                }
                let _ = finished_tx.send(());
            })
            .map_err(Error::Transport)?;
        let (handle, status) = started_rx
            .await
            .map_err(|_| Error::Encoding("MS/TP execution thread failed to start".into()))??;
        Ok((
            Self {
                handle,
                shutdown,
                finished,
            },
            status,
        ))
    }

    pub(super) async fn stop(self) {
        drop(self.shutdown);
        let _ = self.finished.await;
    }
}
