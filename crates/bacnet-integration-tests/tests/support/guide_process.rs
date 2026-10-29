//! Shared child ownership for the exact Rust guide programs: spawn, finish,
//! drop and retirement. Include it as a module named `guide`;
//! `guide_interactive.rs` (stdin and live stdout) and `guide_retirement.rs`
//! (observing retirement) are siblings a test includes only when it uses
//! them.

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, watch};
use tokio::time::{Duration, timeout};

pub const WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct Completion {
    pub output: Output,
    pub cancelled: bool,
}

pub type Retirement = watch::Receiver<Option<Result<Completion, String>>>;

pub struct Process {
    owner: Option<std::thread::JoinHandle<()>>,
    cancel: Option<oneshot::Sender<()>>,
    pub(super) input: Option<oneshot::Sender<Vec<u8>>>,
    pub(super) retired: Retirement,
}

impl Process {
    pub fn spawn(path: &Path, args: &[&str]) -> Result<Self, String> {
        Self::start(path, args, false).map(|(process, _stdout)| process)
    }

    /// Start the child; with `interactive`, its stdin is a pipe fed by one
    /// later input. The receiver follows its stdout as it arrives.
    pub(super) fn start(
        path: &Path,
        args: &[&str],
        interactive: bool,
    ) -> Result<(Self, watch::Receiver<Vec<u8>>), String> {
        let path = path.to_owned();
        let args: Vec<_> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let (cancel, cancelled) = oneshot::channel();
        let (input, incoming) = oneshot::channel();
        let (output, stdout) = watch::channel(Vec::new());
        let (done, retired) = watch::channel(None);
        let (started, ready) = std::sync::mpsc::sync_channel(1);
        // The owner outlives even the caller's runtime, drains both pipes,
        // reaps the child and drops its runtime before reporting retirement.
        let owner = std::thread::Builder::new()
            .name("guide-child".into())
            .spawn(move || {
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run_child(
                        path,
                        args,
                        interactive.then_some(incoming),
                        cancelled,
                        started,
                        output,
                    )),
                    Err(error) => {
                        let error = error.to_string();
                        let _ = started.send(Err(error.clone()));
                        Err(error)
                    }
                };
                done.send_replace(Some(result));
            })
            .map_err(|e| e.to_string())?;
        if let Err(error) = ready
            .recv()
            .map_err(|e| e.to_string())
            .and_then(|value| value)
        {
            let _ = owner.join();
            return Err(error);
        }
        let process = Self {
            owner: Some(owner),
            cancel: Some(cancel),
            input: interactive.then_some(input),
            retired,
        };
        Ok((process, stdout))
    }

    pub async fn finish(mut self) -> Output {
        // Finishing without supplied input means EOF, as from redirected stdin.
        self.input.take();
        let completion = retired(&mut self.retired).await;
        self.cancel.take();
        assert!(!completion.cancelled, "ordinary completion was cancelled");
        completion.output
    }
}

async fn drain(
    mut pipe: impl AsyncRead + Unpin,
    progress: Option<watch::Sender<Vec<u8>>>,
) -> std::io::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = pipe.read(&mut chunk).await?;
        if count == 0 {
            return Ok(result);
        }
        result.extend_from_slice(&chunk[..count]);
        if let Some(progress) = &progress {
            progress.send_replace(result.clone());
        }
    }
}

async fn run_child(
    path: PathBuf,
    args: Vec<String>,
    input: Option<oneshot::Receiver<Vec<u8>>>,
    mut cancellation: oneshot::Receiver<()>,
    started: std::sync::mpsc::SyncSender<Result<(), String>>,
    progress: watch::Sender<Vec<u8>>,
) -> Result<Completion, String> {
    let mut command = tokio::process::Command::new(&path);
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    assert_eq!(command.as_std().get_program(), path.as_os_str());
    let mut child = match command.spawn() {
        Ok(child) => {
            let _ = started.send(Ok(()));
            child
        }
        Err(error) => {
            let error = format!("spawn {}: {error}", path.display());
            let _ = started.send(Err(error.clone()));
            return Err(error);
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdin = child.stdin.take();
    let (stop_input, input_stopped) = oneshot::channel();
    let mut watchdog = false;
    let mut cancelled = false;
    let wait = async {
        let status = tokio::select! {
            biased;
            _ = &mut cancellation => {
                cancelled = true;
                let _ = child.start_kill();
                child.wait().await
            },
            status = child.wait() => status,
            _ = tokio::time::sleep(WAIT) => {
                watchdog = true;
                let _ = child.start_kill();
                child.wait().await
            }
        };
        let _ = stop_input.send(());
        status
    };
    let write = async {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            tokio::select! {
                _ = input_stopped => {},
                received = input => {
                    if let Ok(bytes) = received {
                        stdin.write_all(&bytes).await?;
                    }
                    stdin.shutdown().await?;
                }
            }
        }
        Ok::<_, std::io::Error>(())
    };
    let (status, stdout, stderr, input) = tokio::join!(
        wait,
        timeout(WAIT + WAIT, drain(stdout, Some(progress))),
        timeout(WAIT + WAIT, drain(stderr, None)),
        timeout(WAIT + WAIT, write),
    );
    let status = status.map_err(|e| format!("reap child: {e}"))?;
    let out = stdout
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let err = stderr
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    input
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if watchdog {
        return Err(format!(
            "sample exceeded safety watchdog; status={status}, stdout={out:?}, stderr={err:?}"
        ));
    }
    Ok(Completion {
        output: Output {
            status,
            stdout: out,
            stderr: err,
        },
        cancelled,
    })
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        self.input.take();
        // The dedicated runtime can reap independently while this thread unwinds.
        // Join before returning even if the test itself is about to exit.
        if let Some(owner) = self.owner.take() {
            let _ = owner.join();
        }
    }
}

pub async fn retired(done: &mut Retirement) -> Completion {
    timeout(WAIT + WAIT + WAIT, done.wait_for(Option::is_some))
        .await
        .expect("subprocess owner must retire")
        .expect("subprocess owner must report")
        .as_ref()
        .unwrap()
        .clone()
        .expect("subprocess completion")
}
