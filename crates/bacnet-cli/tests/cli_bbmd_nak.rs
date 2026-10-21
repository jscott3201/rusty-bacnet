//! One-shot `register` and `unregister` fail when the BBMD answers with a
//! NAK, naming it, rather than printing the NAK as a result (#1822).
#[allow(dead_code)]
mod support;

use std::net::{Ipv4Addr, SocketAddr};

use support::{failure, run};
use tokio::net::UdpSocket;

/// A stand-in BBMD on loopback that refuses each registration and FDT
/// entry delete with that request's NAK, or accepts each when `accept`.
async fn mock_bbmd(accept: bool) -> SocketAddr {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let at = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut packet = [0; 64];
        while let Ok((len, from)) = socket.recv_from(&mut packet).await {
            let nak = match packet[..len] {
                [0x81, 0x05, ..] => 0x30,
                [0x81, 0x08, ..] => 0x50,
                _ => continue,
            };
            let code = if accept { 0x00 } else { nak };
            let _ = socket
                .send_to(&[0x81, 0x00, 0x00, 0x06, 0x00, code], from)
                .await;
        }
    });
    at
}

async fn one_shot(command: &str, bbmd: SocketAddr, json: bool) -> std::process::Output {
    let bbmd = bbmd.to_string();
    let mut args = vec!["--interface", "127.0.0.1", "--port", "0"];
    if json {
        args.push("--json");
    }
    if command == "unregister" {
        // Deleting an FDT entry is a write.
        args.push("--allow-write");
    }
    args.extend([command, bbmd.as_str()]);
    run(args).await
}

#[tokio::test]
async fn a_refused_register_fails_and_names_the_nak() {
    let bbmd = mock_bbmd(false).await;
    for json in [false, true] {
        let output = one_shot("register", bbmd, json).await;
        assert_eq!(output.status.code(), Some(1), "json {json}");
        failure(&output, "REGISTER_FOREIGN_DEVICE_NAK");
    }
}

#[tokio::test]
async fn a_refused_unregister_fails_and_names_the_nak() {
    let bbmd = mock_bbmd(false).await;
    for json in [false, true] {
        let output = one_shot("unregister", bbmd, json).await;
        assert_eq!(output.status.code(), Some(1), "json {json}");
        failure(&output, "DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK");
    }
}

#[tokio::test]
async fn an_accepted_register_and_unregister_succeed() {
    let bbmd = mock_bbmd(true).await;
    for (command, line) in [
        ("register", "Register-Foreign-Device accepted"),
        ("unregister", "Delete-Foreign-Device-Table-Entry accepted"),
    ] {
        let output = one_shot(command, bbmd, false).await;
        let (stdout, stderr) = (
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(output.status.success(), "{command}: {stderr}");
        assert!(
            stdout.contains(line) || stderr.contains(line),
            "{command}: {stdout} {stderr}"
        );
    }
}
