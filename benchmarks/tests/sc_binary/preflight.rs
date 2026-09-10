use super::support::*;
use bacnet_benchmarks::sc_helpers::generate_test_certs;
use futures_util::FutureExt;
use std::process::Command;

const HUB: &str = env!("CARGO_BIN_EXE_bacnet-sc-hub");
const DEVICE: &str = env!("CARGO_BIN_EXE_bacnet-device");

async fn failure(cmd: &mut Command, files: &Files, expected: &str) {
    let mut process = Process::start(cmd, files);
    assert!(!process.wait().await.success());
    let (stdout, stderr) = process.output();
    assert!(stdout.is_empty(), "diagnostics must use stderr: {stdout}");
    assert!(stderr.contains(expected), "expected {expected}: {stderr}");
    if expected != "Address already in use" {
        assert!(
            !stderr.contains("Hub bind failed"),
            "preflight reached bind: {stderr}"
        );
        assert!(
            !stderr.contains("WebSocket"),
            "preflight reached networking: {stderr}"
        );
    }
}

#[tokio::test]
async fn discovery_and_retired_flags_are_read_only() {
    let files = Files::new();
    for (bin, retired, flags) in [
        (
            HUB,
            "--self-signed",
            vec!["--ca", "--cert", "--key", "--device-uuid", "--vmac"],
        ),
        (
            DEVICE,
            "--sc-no-verify",
            vec![
                "--sc-ca",
                "--sc-cert",
                "--sc-key",
                "--sc-vmac",
                "--sc-device-uuid",
            ],
        ),
    ] {
        for flag in ["--help", "--version"] {
            let mut process =
                Process::start(Command::new(bin).arg(flag).current_dir(&files.0), &files);
            assert!(process.wait().await.success());
            let (stdout, stderr) = process.output();
            assert!(!stdout.is_empty());
            assert!(stderr.is_empty());
            assert!(!stdout.contains(retired));
            if flag == "--help" {
                for required in &flags {
                    assert!(stdout.contains(required));
                }
            }
        }
        // Missing file and nonsense listen address must not mask migration.
        failure(Command::new(bin).arg(retired), &files, "has been removed").await;
    }
    assert!(!files.0.join("sc-certs").exists());
}

#[tokio::test]
async fn missing_empty_and_invalid_identity_precede_file_or_network_io() {
    let files = Files::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let url = format!("wss://{address}");
    for hub in [true, false] {
        let flags: Vec<(&str, String)> = if hub {
            vec![
                ("--listen", address.clone()),
                ("--ca", "absent".into()),
                ("--cert", "absent".into()),
                ("--key", "absent".into()),
                ("--device-uuid", HUB_UUID_HEX.into()),
            ]
        } else {
            vec![
                ("--transport", "sc".into()),
                ("--sc-hub", url.clone()),
                ("--sc-ca", "absent".into()),
                ("--sc-cert", "absent".into()),
                ("--sc-key", "absent".into()),
                ("--sc-vmac", "020000001388".into()),
                (
                    "--sc-device-uuid",
                    "00000000000000000000000000001388".into(),
                ),
            ]
        };
        for index in 1..flags.len() {
            for empty in [false, true] {
                let mut cmd = Command::new(if hub { HUB } else { DEVICE });
                for (i, (flag, value)) in flags.iter().enumerate() {
                    if i != index {
                        cmd.arg(flag).arg(value);
                    } else if empty {
                        cmd.arg(flag).arg("");
                    }
                }
                failure(&mut cmd, &files, flags[index].0).await;
            }
        }
    }
    for (flag, invalid) in [
        ("--sc-vmac", "000000000000"),
        ("--sc-vmac", "ffffffffffff"),
        ("--sc-vmac", "é0000000000"),
        ("--sc-vmac", "02:00:00:00:13:88"),
        ("--sc-device-uuid", "00000000000000000000000000000000"),
        ("--sc-device-uuid", "zz"),
    ] {
        let base = files.device(&url);
        let mut cmd = replace(&base, flag, invalid);
        failure(&mut cmd, &files, flag).await;
    }
    for (flag, invalid) in [
        ("--device-uuid", "00000000000000000000000000000000"),
        ("--device-uuid", "00000000000000000000000000000001ff"),
        ("--device-uuid", "000000000000000000000000000001"),
        ("--device-uuid", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
        ("--device-uuid", "9a21f164-1a15-454d-9ed7-e3a2710d7001"),
        ("--device-uuid", "é0000000000000000000000000000000"),
        ("--vmac", ""),
        ("--vmac", "000000000000"),
        ("--vmac", "ffffffffffff"),
        ("--vmac", "é0000000000"),
        ("--vmac", "gg0000000001"),
        ("--vmac", "0000000001"),
        ("--vmac", "00000000000001"),
        ("--vmac", "00:00:00:00:00:01"),
    ] {
        let mut base = replace(&files.secure_hub(), "--listen", &address);
        base.args(["--vmac", "000000000001"]);
        failure(&mut replace(&base, flag, invalid), &files, flag).await;
    }
    assert!(listener.accept().now_or_never().is_none());
    // Same executable positive control: valid preflight really dials this oracle.
    files.certs(&generate_test_certs());
    let mut device = Process::start(&mut files.device(&url), &files);
    let (tcp, _) = bounded(listener.accept()).await.unwrap();
    drop(tcp);
    assert!(!device.wait().await.success());
}

pub(super) fn replace(base: &Command, flag: &str, value: &str) -> Command {
    let mut cmd = Command::new(base.get_program());
    let mut args = base.get_args();
    while let Some(arg) = args.next() {
        let text = arg.to_str().unwrap();
        if text == flag {
            let _ = args.next();
            cmd.arg(flag).arg(value);
        } else if text.starts_with(&format!("{flag}=")) {
            cmd.arg(flag).arg(value);
        } else {
            cmd.arg(arg);
        }
    }
    cmd
}

#[tokio::test]
async fn files_der_ca_and_key_match_fail_before_bind_or_dial() {
    let files = Files::new();
    let certs = generate_test_certs();
    files.certs(&certs);
    let other = generate_test_certs();
    let invalid = [
        files.0.join("missing.pem"),
        files.write("empty.pem", ""),
        files.write(
            "malformed.pem",
            "-----BEGIN CERTIFICATE-----\n!!\n-----END CERTIFICATE-----\n",
        ),
        files.write(
            "bad-der.pem",
            "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
        ),
        files.write(
            "mixed-der.pem",
            format!(
                "{}-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
                certs.ca_cert_pem
            ),
        ),
        files.0.clone(),
    ];
    let mismatch = files.write("mismatch.key", other.client_key_pem);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    for hub in [true, false] {
        let base = if hub {
            replace(&files.secure_hub(), "--listen", &address)
        } else {
            files.device(&format!("wss://{address}"))
        };
        for flag in if hub {
            ["--ca", "--cert", "--key"]
        } else {
            ["--sc-ca", "--sc-cert", "--sc-key"]
        } {
            for path in &invalid {
                let mut cmd = replace(&base, flag, path.to_str().unwrap());
                failure(&mut cmd, &files, "Error:").await;
            }
        }
        failure(
            &mut replace(
                &base,
                if hub { "--key" } else { "--sc-key" },
                mismatch.to_str().unwrap(),
            ),
            &files,
            "key",
        )
        .await;
    }
    assert!(listener.accept().now_or_never().is_none());
    // Valid hub reaches bind and reports the actually occupied address.
    failure(
        &mut replace(&files.secure_hub(), "--listen", &address),
        &files,
        "Address already in use",
    )
    .await;
}

#[tokio::test]
async fn non_sc_bip_starts_without_credentials_and_panic_reaps_child() {
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    let files = Files::new();
    let mut cmd = Command::new(DEVICE);
    cmd.args([
        "--interface=127.0.0.1",
        "--broadcast=127.0.0.1",
        "--port=0",
        "--objects=1",
    ]);
    let mut process = Process::start(&mut cmd, &files);
    let line = process.ready("BIP device ").await;
    let mac = line.split_whitespace().nth(2).unwrap();
    let bytes: Vec<u8> = mac
        .split(':')
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect();
    let port = u16::from_be_bytes([bytes[4], bytes[5]]);
    assert!(std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_err());
    let result = AssertUnwindSafe(async move {
        let _owned = process;
        panic!("injected child cleanup failure");
    })
    .catch_unwind()
    .await;
    assert!(result.is_err());
    // UDP has no TIME_WAIT: a successful bind proves child teardown, not a sleep.
    std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
}
