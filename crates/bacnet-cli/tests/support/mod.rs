use std::ffi::OsStr;
use std::fs::{self, File};
use std::future::Future;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(feature = "sc-tls")]
pub mod sc;

static NEXT: AtomicU64 = AtomicU64::new(0);

pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("loopback test exceeded deadline")
}

pub struct Files(pub PathBuf);

impl Files {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cli-sc-required-ca-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        Self(path)
    }

    pub fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove ephemeral credentials/output");
    }
}

pub fn command() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bacnet"));
    cmd.stdin(Stdio::null()).env_remove("RUST_LOG");
    cmd
}

// Files avoid pipe backpressure. The guard owns kill AND wait even on panic or
// timeout; no blocking Command::output on the runtime serving the SC endpoints.
pub struct Process {
    child: Child,
    files: Files,
}

impl Process {
    pub fn spawn(cmd: &mut Command) -> Self {
        let files = Files::new();
        cmd.stdout(File::create(files.0.join("stdout")).unwrap())
            .stderr(File::create(files.0.join("stderr")).unwrap());
        Self {
            child: cmd.spawn().unwrap(),
            files,
        }
    }

    pub async fn output(mut self) -> Output {
        bounded(async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    return Output {
                        status,
                        stdout: fs::read(self.files.0.join("stdout")).unwrap(),
                        stderr: fs::read(self.files.0.join("stderr")).unwrap(),
                    };
                }
                // Process-exit polling only, never a service-readiness barrier.
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.child.wait().expect("reap CLI child");
    }
}

pub async fn run(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Output {
    Process::spawn(command().args(args)).output().await
}

pub fn failure(output: &Output, diagnostic: &str) {
    assert!(!output.status.success(), "unexpected success");
    assert!(output.stdout.is_empty(), "failure emitted stdout");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "expected {diagnostic}: {stderr}"
    );
    assert!(!stderr.contains("PRIVATE KEY"), "credential leaked");
}
