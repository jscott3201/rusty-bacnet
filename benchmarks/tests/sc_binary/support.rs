#![allow(dead_code)]

use bacnet_benchmarks::sc_helpers::CertMaterial;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const DEADLINE: Duration = Duration::from_secs(10);
// Fixed test provisioning, not deployment defaults. Also used by Compose smoke.
pub const HUB_UUID_HEX: &str = "9a21f1641a15454d9ed7e3a2710d7001";
pub const HUB_UUID: [u8; 16] = [
    0x9a, 0x21, 0xf1, 0x64, 0x1a, 0x15, 0x45, 0x4d, 0x9e, 0xd7, 0xe3, 0xa2, 0x71, 0x0d, 0x70, 1,
];
pub const HUB_VMAC: [u8; 6] = [0, 0, 0, 0, 0, 1];
static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Files(pub PathBuf);

impl Files {
    pub fn new() -> Self {
        let parent = std::env::var_os("SC_TEST_ARTIFACTS")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = parent.join(format!(
            "docker-sc-mtls-{}-{}-{:016x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            rand::random::<u64>()
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

    pub fn write(&self, name: &str, data: impl AsRef<[u8]>) -> PathBuf {
        use std::io::Write;
        let path = self.0.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .unwrap()
            .write_all(data.as_ref())
            .unwrap();
        path
    }

    pub fn certs(&self, certs: &CertMaterial) {
        self.write("ca.pem", &certs.ca_cert_pem);
        self.write("hub.pem", &certs.server_cert_pem);
        self.write("hub.key", &certs.server_key_pem);
        self.write("device.pem", &certs.client_cert_pem);
        self.write("device.key", &certs.client_key_pem);
    }

    pub fn hub(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_bacnet-sc-hub"));
        cmd.args(["--listen", "127.0.0.1:0"])
            .args(["--device-uuid", HUB_UUID_HEX])
            .arg("--cert")
            .arg(self.0.join("hub.pem"))
            .arg("--key")
            .arg(self.0.join("hub.key"));
        cmd
    }

    pub fn secure_hub(&self) -> Command {
        let mut cmd = self.hub();
        cmd.arg("--ca").arg(self.0.join("ca.pem"));
        cmd
    }

    pub fn device(&self, url: &str) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_bacnet-device"));
        cmd.args([
            "--transport=sc",
            "--objects=1",
            "--device-instance=5000",
            "--sc-hub",
            url,
            "--sc-vmac=020000001388",
            "--sc-device-uuid=00000000000000000000000000001388",
        ])
        .arg("--sc-ca")
        .arg(self.0.join("ca.pem"))
        .arg("--sc-cert")
        .arg(self.0.join("device.pem"))
        .arg("--sc-key")
        .arg(self.0.join("device.key"));
        cmd
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        if std::env::var_os("SC_TEST_ARTIFACTS").is_none() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

// Regular files avoid pipe backpressure and reader tasks. Drop always kills AND
// reaps, including assertion panic and deadline expiry; never detach a child.
pub struct Process {
    pub child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Process {
    pub fn start(cmd: &mut Command, files: &Files) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let stdout = files.write(&format!("{id}.stdout"), "");
        let stderr = files.write(&format!("{id}.stderr"), "");
        let child = cmd
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        Self {
            child,
            stdout,
            stderr,
        }
    }

    pub fn output(&self) -> (String, String) {
        let stdout = fs::read_to_string(&self.stdout).unwrap();
        let stderr = fs::read_to_string(&self.stderr).unwrap();
        for text in [&stdout, &stderr] {
            assert!(
                !text.contains("PRIVATE KEY"),
                "private key marker in output"
            );
        }
        (stdout, stderr)
    }

    pub async fn wait(&mut self) -> ExitStatus {
        let end = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < end,
                "child did not exit: {:?}",
                self.output()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn ready(&mut self, marker: &str) -> String {
        let end = Instant::now() + DEADLINE;
        loop {
            let (_, stderr) = self.output();
            // A regular file can be observed midway through eprintln!'s writes.
            // Never parse a partial readiness line as an address/VMAC.
            if let Some(line) = stderr
                .split_inclusive('\n')
                .find(|line| line.ends_with('\n') && line.contains(marker))
            {
                return line.trim_end_matches('\n').to_owned();
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "child exited: {stderr}"
            );
            assert!(Instant::now() < end, "no readiness barrier: {stderr}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn hub_url(&mut self) -> String {
        let line = self.ready("BACnet/SC hub listening on ").await;
        format!("wss://{}", line.split("listening on ").nth(1).unwrap())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(DEADLINE, future)
        .await
        .expect("operation deadline expired")
}

pub fn pem(path: &Path) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::CertificateDer::pem_file_iter(path)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
