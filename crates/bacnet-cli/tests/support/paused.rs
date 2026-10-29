//! Owned blocking work for subprocess tests whose server clock must stay fixed.
use super::Process;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

pub struct BlockingGuard {
    release: Option<Sender<()>>,
    retired: Receiver<()>,
}

struct Retired(Sender<()>);
impl Drop for Retired {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

impl BlockingGuard {
    fn start(work: impl FnOnce(Receiver<()>) + Send + 'static) -> Self {
        let (release, released) = mpsc::channel();
        let (retired, done) = mpsc::channel();
        tokio::task::spawn_blocking(move || {
            let _retired = Retired(retired);
            work(released);
        });
        Self {
            release: Some(release),
            retired: done,
        }
    }

    /// An entered blocking task prevents Tokio's paused clock auto-advancing.
    pub async fn hold_clock() -> Self {
        let (entered, ready) = tokio::sync::oneshot::channel();
        let guard = Self::start(move |released| {
            let _ = entered.send(());
            let _ = released.recv();
        });
        ready.await.expect("clock holder entered");
        guard
    }
}

impl Drop for BlockingGuard {
    fn drop(&mut self) {
        let _ = self.release.take().unwrap().send(());
        let _ = self.retired.recv();
    }
}

/// Completion independent of the paused parent clock. The existing ten-second
/// harness bound uses wall time; the CLI's own deadline is unchanged.
/// Cancellation and panic release the worker, which kills and reaps its owned
/// Process before acknowledging retirement. Abort alone cannot retire it.
pub async fn output(process: Process) -> std::process::Output {
    run(process, None).await
}

/// Expose worker entry and its owned output directory to cleanup assertions.
pub async fn output_started(
    process: Process,
    entered: tokio::sync::oneshot::Sender<std::path::PathBuf>,
) -> std::process::Output {
    run(process, Some(entered)).await
}

async fn run(
    mut process: Process,
    entered: Option<tokio::sync::oneshot::Sender<std::path::PathBuf>>,
) -> std::process::Output {
    let (result, completed) = tokio::sync::oneshot::channel();
    let guard = BlockingGuard::start(move |cancelled| {
        if let Some(entered) = entered {
            let _ = entered.send(process.files.0.clone());
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let value = loop {
            if let Some(status) = process.child.try_wait().expect("poll CLI child") {
                break Ok(std::process::Output {
                    status,
                    stdout: std::fs::read(process.files.0.join("stdout")).unwrap(),
                    stderr: std::fs::read(process.files.0.join("stderr")).unwrap(),
                });
            }
            if Instant::now() >= deadline {
                break Err("loopback test exceeded deadline");
            }
            // Process-exit polling only, never a service or limiter barrier.
            if cancelled.recv_timeout(Duration::from_millis(5))
                != Err(mpsc::RecvTimeoutError::Timeout)
            {
                return;
            }
        };
        drop(process);
        let _ = result.send(value);
    });
    let value = completed.await.expect("process worker completed");
    drop(guard);
    value.expect("CLI completed inside harness deadline")
}
