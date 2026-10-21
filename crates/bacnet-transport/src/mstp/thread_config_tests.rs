//! The dedicated thread's scheduling settings (#501): what configuration
//! refuses, what `start` reports, and on Linux that a setting the kernel
//! won't apply fails `start` and keeps the serial port.

use super::test_support::config;
use super::{
    LoopbackSerial, MstpExecutionMode, MstpExecutionStatus, MstpThreadConfig, MstpTransport,
    MAX_CPUS, MAX_REALTIME_PRIORITY, MIN_REALTIME_PRIORITY,
};
use crate::port::TransportPort;

fn transport(mode: MstpExecutionMode) -> MstpTransport<LoopbackSerial> {
    let (serial, _peer) = LoopbackSerial::pair();
    MstpTransport::new(serial, config(3, 127)).with_execution_mode(mode)
}

#[test]
fn out_of_range_settings_are_refused_on_every_os() {
    for priority in [0, MAX_REALTIME_PRIORITY + 1, u8::MAX] {
        let error = MstpThreadConfig::new()
            .with_realtime_priority(priority)
            .validate()
            .unwrap_err();
        assert!(
            error.to_string().contains("realtime_priority"),
            "{priority}: {error}"
        );
    }
    let error = MstpThreadConfig::new()
        .with_cpu_affinity([0, MAX_CPUS])
        .validate()
        .unwrap_err();
    assert!(error.to_string().contains("CPU 1024"), "{error}");
    assert!(MstpExecutionMode::Tokio.validate().is_ok());
    assert!(MstpExecutionMode::dedicated_thread().validate().is_ok());
}

#[cfg(target_os = "linux")]
#[test]
fn in_range_settings_pass_on_linux() {
    for priority in [MIN_REALTIME_PRIORITY, MAX_REALTIME_PRIORITY] {
        let config = MstpThreadConfig::new().with_realtime_priority(priority);
        assert!(config.validate().is_ok(), "{priority}");
    }
    let config = MstpThreadConfig::new().with_cpu_affinity([0, MAX_CPUS - 1]);
    assert!(config.validate().is_ok());
    assert_eq!(nix::sched::CpuSet::count(), MAX_CPUS);
}

/// Elsewhere both settings are refused, by `validate` and by `start`,
/// which keeps the serial port; none is ignored.
#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn settings_are_refused_off_linux() {
    let refused = [
        MstpThreadConfig::new().with_realtime_priority(MIN_REALTIME_PRIORITY),
        MstpThreadConfig::new().with_cpu_affinity([0]),
    ];
    for thread in refused {
        let mode = MstpExecutionMode::DedicatedThread(thread);
        let error = mode.validate().unwrap_err();
        assert!(error.to_string().contains("Linux only"), "{error}");
        let bacnet_types::error::Error::Transport(io) = &error else {
            panic!("not an I/O error: {error}");
        };
        assert_eq!(io.kind(), std::io::ErrorKind::Unsupported);
        let mut transport = transport(mode);
        let error = transport.start().await.unwrap_err();
        assert!(error.to_string().contains("Linux only"), "{error}");
        assert_eq!(transport.execution_status(), None);
        // The serial port is still there: a plain thread starts.
        let mut transport = transport.with_execution_mode(MstpExecutionMode::dedicated_thread());
        let _npdus = transport.start().await.unwrap();
        transport.stop().await.unwrap();
    }
}

#[tokio::test]
async fn start_reports_how_the_loop_runs() {
    let plain_thread = MstpExecutionStatus {
        dedicated_thread: true,
        ..MstpExecutionStatus::default()
    };
    for (mode, expected) in [
        (MstpExecutionMode::Tokio, MstpExecutionStatus::default()),
        (MstpExecutionMode::dedicated_thread(), plain_thread),
    ] {
        let mut transport = transport(mode);
        let diagnostics = transport.diagnostics();
        assert_eq!(transport.execution_status(), None, "before start");
        let _npdus = transport.start().await.unwrap();
        assert_eq!(transport.execution_status(), Some(expected.clone()));
        transport.stop().await.unwrap();
        drop(transport);
        assert_eq!(diagnostics.execution_status(), Some(expected));
    }
}

/// Without `CAP_SYS_NICE` or an `RLIMIT_RTPRIO` (CI's runners have
/// neither) even `SCHED_FIFO` 1 is refused, and `start` says so and keeps
/// the serial port. Where the host does allow it, the status must show it,
/// and only the transport's thread runs real-time. Either way a started
/// transport never runs without what it asked for.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_refused_realtime_priority_fails_start() {
    let thread = MstpThreadConfig::new().with_realtime_priority(MIN_REALTIME_PRIORITY);
    let mut transport = transport(MstpExecutionMode::DedicatedThread(thread));
    match transport.start().await {
        Err(error) => {
            let bacnet_types::error::Error::Transport(io) = &error else {
                panic!("not an I/O error: {error}");
            };
            assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied, "{error}");
            assert!(error.to_string().contains("CAP_SYS_NICE"), "{error}");
            assert_eq!(transport.execution_status(), None);
            let mut transport =
                transport.with_execution_mode(MstpExecutionMode::dedicated_thread());
            let _npdus = transport.start().await.unwrap();
            transport.stop().await.unwrap();
        }
        Ok(_npdus) => {
            let status = transport.execution_status().unwrap();
            assert_eq!(status.realtime_priority, Some(MIN_REALTIME_PRIORITY));
            let (fifo, _) = crate::sys::sched::current().unwrap();
            assert!(!fifo, "the test's own thread went real-time");
            transport.stop().await.unwrap();
        }
    }
}

/// The CPUs this process may run on.
#[cfg(target_os = "linux")]
fn allowed_cpus() -> Vec<usize> {
    use nix::sched::{sched_getaffinity, CpuSet};
    let set = sched_getaffinity(nix::unistd::Pid::from_raw(0)).unwrap();
    (0..CpuSet::count())
        .filter(|cpu| set.is_set(*cpu).unwrap())
        .collect()
}

/// Pinning needs no privilege. The first CPU the test may use (CPU 0 on
/// CI's runners), named twice, is applied once and read back.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn affinity_to_an_allowed_cpu_is_applied() {
    let before = allowed_cpus();
    let cpu = before[0];
    let thread = MstpThreadConfig::new().with_cpu_affinity([cpu, cpu]);
    let mut transport = transport(MstpExecutionMode::DedicatedThread(thread));
    let _npdus = transport.start().await.unwrap();
    let status = transport.execution_status().unwrap();
    assert_eq!(
        status,
        MstpExecutionStatus {
            dedicated_thread: true,
            realtime_priority: None,
            cpu_affinity: vec![cpu],
        }
    );
    transport.stop().await.unwrap();
    // Only the transport's thread was pinned.
    assert_eq!(allowed_cpus(), before);
}

/// A CPU the process may not use fails `start` and keeps the serial port:
/// alone, the kernel refuses the set; beside an allowed one, it applies
/// only that one, which the read-back finds.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn affinity_to_a_cpu_outside_the_set_fails_start() {
    let allowed = allowed_cpus();
    let Some(outside) = (0..MAX_CPUS).rev().find(|cpu| !allowed.contains(cpu)) else {
        return; // Every CPU number is allowed; nothing to name.
    };
    for (cpus, expected) in [
        (vec![outside], "setting CPU affinity"),
        (vec![allowed[0], outside], "the kernel applied"),
    ] {
        let thread = MstpThreadConfig::new().with_cpu_affinity(cpus);
        let mut transport = transport(MstpExecutionMode::DedicatedThread(thread));
        let error = transport.start().await.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(transport.execution_status(), None);
        let mut transport = transport.with_execution_mode(MstpExecutionMode::dedicated_thread());
        let _npdus = transport.start().await.unwrap();
        transport.stop().await.unwrap();
    }
}
