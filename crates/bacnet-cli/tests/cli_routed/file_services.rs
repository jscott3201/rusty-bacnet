//! Routed file windows preserve binary content, record boundaries and staged output.
use super::file_fixture::{STREAM, Window, database, oid, records};
use super::*;
use support::Files;

async fn invoke(
    port: u16,
    target: &str,
    service: &str,
    arguments: &[&str],
) -> std::process::Output {
    invoke_to(Some(port), target, service, arguments).await
}

async fn invoke_to(
    port: Option<u16>,
    target: &str,
    service: &str,
    arguments: &[&str],
) -> std::process::Output {
    let mut cmd = command();
    cmd.args(["--interface", "127.0.0.1", "--port", "0", "--json"]);
    if let Some(port) = port {
        cmd.args(["--foreign-bbmd", &format!("127.0.0.1:{port}")]);
    }
    cmd.arg(service).arg(target).args(arguments);
    Process::spawn(&mut cmd).output().await
}

fn success(output: &std::process::Output, message: String) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"status":"ok", "message":message})
    );
}

#[tokio::test]
async fn routed_stream_file_reads_every_window_and_final_eof() {
    let (db, observations) = database();
    let (mut router, mut server, port) = site_with_database(db).await;
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let files = Files::new();
        let path = files.0.join("payload.bin");
        let output = invoke(
            port,
            &target,
            "file-read",
            &["10", "--count", "2", "--output", path.to_str().unwrap()],
        )
        .await;
        success(&output, format!("Wrote 7 bytes to {}", path.display()));
        assert_eq!(std::fs::read(&path).unwrap(), STREAM);
        assert_eq!(
            *observations.lock().unwrap(),
            vec![
                Window {
                    instance: 10,
                    start: 0,
                    requested: 2,
                    returned: Some((2, false))
                },
                Window {
                    instance: 10,
                    start: 2,
                    requested: 2,
                    returned: Some((2, false))
                },
                Window {
                    instance: 10,
                    start: 4,
                    requested: 2,
                    returned: Some((2, false))
                },
                Window {
                    instance: 10,
                    start: 6,
                    requested: 2,
                    returned: Some((1, true))
                },
            ]
        );
        observations.lock().unwrap().clear();
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_record_file_keeps_empty_binary_and_unequal_records() {
    let (db, observations) = database();
    let (mut router, mut server, port) = site_with_database(db).await;
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let files = Files::new();
        let path = files.0.join("records");
        let output = invoke(
            port,
            &target,
            "file-read",
            &[
                "11",
                "--access",
                "record",
                "--count",
                "2",
                "--output",
                path.to_str().unwrap(),
            ],
        )
        .await;
        success(
            &output,
            format!("Wrote 5 records (10 bytes) to {}", path.display()),
        );
        let mut names = std::fs::read_dir(&path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            (0..5)
                .map(|index| format!("record-{index:010}.bin"))
                .collect::<Vec<_>>()
        );
        for (index, record) in records().iter().enumerate() {
            assert_eq!(
                std::fs::read(path.join(format!("record-{index:010}.bin"))).unwrap(),
                *record
            );
        }
        assert_eq!(
            *observations.lock().unwrap(),
            vec![
                Window {
                    instance: 11,
                    start: 0,
                    requested: 2,
                    returned: Some((2, false))
                },
                Window {
                    instance: 11,
                    start: 2,
                    requested: 2,
                    returned: Some((2, false))
                },
                Window {
                    instance: 11,
                    start: 4,
                    requested: 2,
                    returned: Some((1, true))
                },
            ]
        );
        observations.lock().unwrap().clear();
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_file_write_preserves_json_and_remote_bytes_and_refusal() {
    let (db, _) = database();
    let (mut router, mut server, port) = site_with_database(db).await;
    for (target, bytes) in [
        (format!("{DNET}:{INSTANCE}"), &[0x12, 0, 0xff, 0x23][..]),
        (format!("{DNET}:0x05"), &[0x80, 0x42, 0, 0x91][..]),
    ] {
        let files = Files::new();
        let input = files.write("input.bin", bytes);
        let args = [
            "10",
            input.to_str().unwrap(),
            "--start",
            "2",
            "--allow-write",
        ];
        let output = invoke(port, &target, "file-write", &args).await;
        success(&output, format!("Wrote 4 bytes from {}", input.display()));
        {
            let db = server.database().read().await;
            let data = db
                .get(&oid(10))
                .unwrap()
                .file_storage_internal()
                .unwrap()
                .read_stream(0, 20)
                .unwrap();
            let mut expected = STREAM.to_vec();
            expected[2..6].copy_from_slice(bytes);
            assert_eq!(data.data, expected);
            assert!(data.end_of_file);
        }
        let output = invoke(
            port,
            &target,
            "file-write",
            &["13", input.to_str().unwrap(), "--allow-write"],
        )
        .await;
        failure(&output, "FILE_ACCESS_DENIED");
        assert_eq!(
            server
                .database()
                .read()
                .await
                .get(&oid(13))
                .unwrap()
                .file_storage_internal()
                .unwrap()
                .read_stream(0, 20)
                .unwrap()
                .data,
            STREAM
        );
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_later_file_window_failure_removes_partial_stream_and_record_output() {
    let (db, observations) = database();
    let (mut router, mut server, port) = site_with_database(db).await;
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        for (instance, access) in [(12, "stream"), (14, "record")] {
            let files = Files::new();
            let path = files.0.join("output");
            let output = invoke(
                port,
                &target,
                "file-read",
                &[
                    &instance.to_string(),
                    "--access",
                    access,
                    "--count",
                    "2",
                    "--output",
                    path.to_str().unwrap(),
                ],
            )
            .await;
            failure(&output, "FILE_ACCESS_DENIED");
            assert!(!path.exists());
            assert_eq!(
                std::fs::read_dir(&files.0).unwrap().count(),
                0,
                "partial staging output remains"
            );
            assert_eq!(
                *observations.lock().unwrap(),
                vec![
                    Window {
                        instance,
                        start: 0,
                        requested: 2,
                        returned: Some((2, false))
                    },
                    Window {
                        instance,
                        start: 2,
                        requested: 2,
                        returned: None
                    },
                ]
            );
            observations.lock().unwrap().clear();
        }
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_file_write_without_permission_precedes_discovery_and_registration() {
    use std::io::ErrorKind;
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket.set_nonblocking(true).unwrap();
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let output = invoke(
            socket.local_addr().unwrap().port(),
            &target,
            "file-write",
            &["10", "no-such-input.bin"],
        )
        .await;
        assert_eq!(output.status.code(), Some(4));
        assert!(output.stdout.is_empty());
        let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(refusal["error"], "refused");
        assert_eq!(refusal["reason"], "allow-write-required");
        assert_eq!(refusal["command"], "file-write");
        assert_eq!(refusal["class"], "write");
        assert_eq!(refusal["missing"], json!(["--allow-write"]));
        assert_eq!(
            socket.recv_from(&mut [0; 1500]).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
    }
}

#[tokio::test]
async fn direct_file_commands_keep_their_existing_local_destination() {
    let (db, _) = database();
    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(INSTANCE))
        .database(db)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    let files = Files::new();
    let stream_path = files.0.join("stream.bin");
    let output = invoke_to(
        None,
        &target,
        "file-read",
        &[
            "10",
            "--count",
            "2",
            "--output",
            stream_path.to_str().unwrap(),
        ],
    )
    .await;
    success(
        &output,
        format!("Wrote 7 bytes to {}", stream_path.display()),
    );
    assert_eq!(std::fs::read(&stream_path).unwrap(), STREAM);
    let record_path = files.0.join("records");
    let output = invoke_to(
        None,
        &target,
        "file-read",
        &[
            "11",
            "--access",
            "record",
            "--count",
            "2",
            "--output",
            record_path.to_str().unwrap(),
        ],
    )
    .await;
    success(
        &output,
        format!("Wrote 5 records (10 bytes) to {}", record_path.display()),
    );
    for (index, record) in records().iter().enumerate() {
        assert_eq!(
            std::fs::read(record_path.join(format!("record-{index:010}.bin"))).unwrap(),
            *record
        );
    }
    let input = files.write("input.bin", [0x35, 0, 0xff]);
    let output = invoke_to(
        None,
        &target,
        "file-write",
        &["10", input.to_str().unwrap(), "--allow-write"],
    )
    .await;
    success(&output, format!("Wrote 3 bytes from {}", input.display()));
    let data = server
        .database()
        .read()
        .await
        .get(&oid(10))
        .unwrap()
        .file_storage_internal()
        .unwrap()
        .read_stream(0, 20)
        .unwrap();
    assert_eq!(data.data, [0x35, 0, 0xff, 0xff, 0, 0x31, 0xa5]);
    server.stop().await.unwrap();
}
