//! A failed BACnet/SC dial's fatal error line ends with its error code by
//! name (#2438): a one-shot command's, and `bacnet mcp serve`'s on an SC
//! profile, since that server reports a failed dial the same way.
#![cfg(feature = "sc-tls")]
#[allow(dead_code)]
mod support;

use rcgen::ExtendedKeyUsagePurpose::ClientAuth;
use support::sc::{Site, cli, read};
use support::{Files, Process};

/// The line a refused dial ends with.
const REFUSED: &str = "(BACnet/SC error code TCP_CONNECTION_REFUSED)";

/// A loopback address nothing listens on. The listener is dropped, so
/// another process could take the port before the dial (and Windows can
/// fail a loopback connect on a local port collision): callers go again
/// when the dial was not refused.
fn closed_port() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

#[tokio::test]
async fn a_refused_dial_names_its_error_code() {
    let site = Site::new();
    let files = Files::new();
    let ca = files.write("site.pem", site.ca.pem());
    let leaf = site.leaf("cli", ClientAuth, None);
    let mut seen = Vec::new();
    for _ in 0..5 {
        let mut cmd = cli(&files, &format!("wss://{}", closed_port()), &leaf);
        cmd.arg("--sc-ca").arg(&ca);
        let output = Process::spawn(read(&mut cmd)).output().await;
        assert!(!output.status.success(), "a refused dial succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let line = stderr
            .lines()
            .find(|line| line.starts_with("Error: "))
            .unwrap_or_default()
            .trim_end_matches('\r');
        if line.contains("tcp-refused") {
            assert!(line.ends_with(REFUSED), "{line}");
            return;
        }
        seen.push(stderr);
    }
    panic!("no dial was refused: {seen:#?}");
}

/// `bacnet mcp serve` on an SC profile whose hub refuses the connection
/// exits with the same line. Authorizing the profile needs a terminal, so
/// this runs where a pseudo-terminal is at hand.
#[cfg(all(feature = "mcp", unix))]
#[test]
fn mcp_serve_on_a_refused_hub_names_the_error_code() {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    let site = Site::new();
    let files = Files::new();
    let ca = files.write("site.pem", site.ca.pem());
    let leaf = site.leaf("mcp", ClientAuth, None);
    let cert = files.write("node.pem", leaf.cert.pem());
    let key = files.write("node.key", leaf.key.serialize_pem());
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config.path().join("profiles")).unwrap();
    let bacnet = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bacnet"));
        command
            .env_remove("RUST_LOG")
            .env("BACNET_CONFIG_DIR", config.path());
        command
    };
    let mut seen = Vec::new();
    for _ in 0..5 {
        let profile = format!(
            "version = 1\ntransport = \"sc\"\n\n[sc]\nhub_url = \"wss://{}\"\nca = '{}'\n\
             cert = '{}'\nkey = '{}'\nvmac = \"02:00:00:00:00:51\"\n\
             device_uuid = \"5b0d6a1e-3c7f-4e2a-9d81-c4f0e6b2a751\"\nreconnect = false\n\n\
             [safety]\nwrites = \"never\"\ndangerous = false\n\n[mcp]\nnetworks = [0]\n",
            closed_port(),
            ca.display(),
            cert.display(),
            key.display()
        );
        std::fs::write(config.path().join("profiles/site.toml"), profile).unwrap();
        // A person authorizes the profile at a terminal, as for any server.
        let pty = nix::pty::openpty(None, None).expect("openpty");
        let mut child = bacnet()
            .args(["mcp", "authorize", "--profile", "site"])
            .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
            .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
            .stderr(Stdio::from(pty.slave))
            .spawn()
            .unwrap();
        let mut master = std::fs::File::from(pty.master);
        master.write_all(b"site\n").unwrap();
        let reader = std::thread::spawn(move || {
            let mut shown = Vec::new();
            let mut buffer = [0; 4096];
            // Linux reports EIO, not end of file, once the slave is closed.
            while let Ok(read @ 1..) = master.read(&mut buffer) {
                shown.extend_from_slice(&buffer[..read]);
            }
            String::from_utf8_lossy(&shown).into_owned()
        });
        let status = child.wait().unwrap();
        let shown = reader.join().unwrap();
        assert!(status.success(), "authorize failed: {shown}");

        let output = bacnet()
            .args(["mcp", "serve", "--stdio", "--profile", "site"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty(), "a failed dial wrote to stdout");
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let line = stderr
            .lines()
            .find(|line| line.starts_with("Error: "))
            .unwrap_or_default()
            .to_owned();
        if line.contains("tcp-refused") {
            assert!(line.ends_with(REFUSED), "{line}");
            return;
        }
        seen.push(stderr);
    }
    panic!("no dial was refused: {seen:#?}");
}
