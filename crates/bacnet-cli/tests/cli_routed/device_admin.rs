//! Routed administrative commands reach the remote handlers with existing guards.
use super::*;
use bacnet_server::server::{DccPolicy, DccState, ServerConfig};
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_types::enums::ReinitializedState;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

async fn invoke(
    port: u16,
    target: &str,
    service: &str,
    arguments: &[&str],
) -> std::process::Output {
    let mut cmd = command();
    cmd.env("BACNET_CLI_ROUTED_WAIT_MS", "300")
        .args([
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--json",
            "--foreign-bbmd",
            &format!("127.0.0.1:{port}"),
            service,
            target,
        ])
        .args(arguments);
    Process::spawn(&mut cmd).output().await
}

fn success(output: &std::process::Output, message: &str) {
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
async fn routed_control_and_reinit_reach_password_protected_handlers() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let received = observed.clone();
    let config = ServerConfig {
        device: lab_identity(INSTANCE),
        dcc_policy: DccPolicy::RequirePassword,
        dcc_password: Some("lab-pass".into()),
        reinit_password: Some("lab-pass".into()),
        on_reinitialize: Some(Arc::new(move |context, _| {
            received.lock().unwrap().push(context.state);
            Ok(())
        })),
        ..Default::default()
    };
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        // Each target has an independent BBMD registration budget.
        let (mut router, mut server, port) =
            site_with_config(lab_database(INSTANCE), config.clone()).await;
        success(
            &invoke(
                port,
                &target,
                "control",
                &[
                    "disable-initiation",
                    "--duration",
                    "5",
                    "--password",
                    "lab-pass",
                    "--allow-write",
                    "--yes",
                ],
            )
            .await,
            "OK",
        );
        assert_eq!(server.comm_state(), DccState::DisableInitiation);
        success(
            &invoke(
                port,
                &target,
                "control",
                &["enable", "--password", "lab-pass", "--allow-write", "--yes"],
            )
            .await,
            "OK",
        );
        assert_eq!(server.comm_state(), DccState::Enable);
        let bad = invoke(
            port,
            &target,
            "control",
            &[
                "disable-initiation",
                "--password",
                "wrong",
                "--allow-write",
                "--yes",
            ],
        )
        .await;
        failure(&bad, "PASSWORD_FAILURE");
        assert_eq!(server.comm_state(), DccState::Enable);
        let before = observed.lock().unwrap().len();
        let bad = invoke(
            port,
            &target,
            "reinit",
            &["warmstart", "--password", "wrong", "--allow-write", "--yes"],
        )
        .await;
        failure(&bad, "PASSWORD_FAILURE");
        assert_eq!(observed.lock().unwrap().len(), before);
        success(
            &invoke(
                port,
                &target,
                "reinit",
                &[
                    "warmstart",
                    "--password",
                    "lab-pass",
                    "--allow-write",
                    "--yes",
                ],
            )
            .await,
            "OK",
        );
        assert_eq!(observed.lock().unwrap().len(), before + 1);
        assert_eq!(
            observed.lock().unwrap().last(),
            Some(&ReinitializedState::WARMSTART)
        );
        server.stop().await.unwrap();
        router.stop().await.unwrap();
    }
}

#[tokio::test]
async fn routed_local_and_utc_time_sync_are_observed_at_remote_clock() {
    let (sender, mut received) = mpsc::unbounded_channel();
    let config = ServerConfig {
        device: lab_identity(INSTANCE),
        on_time_sync: Some(Arc::new(move |data| {
            sender.send(data).unwrap();
        })),
        ..Default::default()
    };
    let (mut router, mut server, port) = site_with_config(lab_database(INSTANCE), config).await;
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        for utc in [false, true] {
            let flags = if utc {
                vec!["--allow-write", "--utc"]
            } else {
                vec!["--allow-write"]
            };
            success(
                &invoke(port, &target, "time-sync", &flags).await,
                "Time synchronized",
            );
            let observed =
                tokio::time::timeout(std::time::Duration::from_secs(10), received.recv())
                    .await
                    .expect("remote time-sync callback watchdog")
                    .unwrap();
            assert_eq!(observed.is_utc, utc);
            assert_eq!(observed.source_network.unwrap().network, 1);
            let request = TimeSynchronizationRequest::decode(&observed.raw_service_data).unwrap();
            assert!((1..=12).contains(&request.date.month));
            assert!((1..=31).contains(&request.date.day));
            assert!((1..=7).contains(&request.date.day_of_week));
            assert!(request.time.hour < 24 && request.time.minute < 60);
            assert!(request.time.second < 60 && request.time.hundredths < 100);
        }
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_admin_guards_refuse_before_registration_or_discovery() {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket.set_nonblocking(true).unwrap();
    let port = socket.local_addr().unwrap().port();
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        for (service, args) in [
            ("control", vec!["enable"]),
            ("reinit", vec!["warmstart"]),
            ("time-sync", vec![]),
        ] {
            let output = invoke(port, &target, service, &args).await;
            assert_eq!(output.status.code(), Some(4));
            assert!(output.stdout.is_empty());
            let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(refusal["reason"], "allow-write-required");
            if service != "time-sync" {
                let mut args = args.clone();
                args.push("--allow-write");
                let output = invoke(port, &target, service, &args).await;
                assert_eq!(output.status.code(), Some(4));
                let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
                assert_eq!(refusal["reason"], "yes-required");
            }
            assert_eq!(
                socket.recv_from(&mut [0; 1500]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}

#[tokio::test]
async fn routed_admin_rejects_malformed_and_unresolved_targets() {
    let (mut router, mut server, port) = site().await;
    for (service, args) in [
        ("control", vec!["enable", "--allow-write", "--yes"]),
        ("reinit", vec!["warmstart", "--allow-write", "--yes"]),
        ("time-sync", vec!["--allow-write"]),
    ] {
        let output = invoke(port, "2:0x", service, &args).await;
        failure(&output, "invalid MAC in routed target");
        let output = invoke(port, "999999", service, &args).await;
        failure(&output, "not found");
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn direct_local_and_utc_time_sync_keep_existing_output_and_delivery() {
    let (sender, mut received) = mpsc::unbounded_channel();
    let config = ServerConfig {
        device: lab_identity(INSTANCE),
        on_time_sync: Some(Arc::new(move |data| {
            sender.send(data).unwrap();
        })),
        ..Default::default()
    };
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut server = BACnetServer::start(config, lab_database(INSTANCE), transport)
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    for utc in [false, true] {
        let mut args = vec![
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--json",
            "time-sync",
            &target,
            "--allow-write",
        ];
        if utc {
            args.push("--utc");
        }
        success(&run(args).await, "Time synchronized");
        let observed = tokio::time::timeout(std::time::Duration::from_secs(10), received.recv())
            .await
            .expect("direct time-sync callback watchdog")
            .unwrap();
        assert_eq!(observed.is_utc, utc);
        assert!(observed.source_network.is_none());
        TimeSynchronizationRequest::decode(&observed.raw_service_data).unwrap();
    }
    server.stop().await.unwrap();
}
