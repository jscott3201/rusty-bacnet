use super::*;
use bytes::BytesMut;
use tokio::time::{timeout, Duration};

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempBdtFile {
    path: PathBuf,
}

impl TempBdtFile {
    fn new(label: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusty-bacnet-{label}-{}-{suffix}.bdt",
            std::process::id()
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempBdtFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

async fn raw_bvlc_request(target: &[u8], function: BvlcFunction, payload: &[u8]) -> BvllMessage {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let (ip, port) = decode_bip_mac(target).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

    let buf = raw_frame(function, payload);
    socket.send_to(&buf, dest).await.unwrap();

    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLC response")
        .unwrap();
    decode_bvll(&recv_buf[..len]).unwrap()
}

#[tokio::test]
async fn seeded_persisted_bdt_loads_on_startup_and_network_write_bdt_mutates_neither_memory_nor_disk(
) {
    let persist = TempBdtFile::new("bdt-restart");
    let fallback_entry = BdtEntry {
        ip: [10, 20, 30, 40],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let seeded_entry = BdtEntry {
        ip: [192, 0, 2, 44],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };

    // Seed a valid wire-format BDT file directly; network Write-BDT no longer
    // creates or updates this file.
    let mut seed_buf = BytesMut::new();
    bbmd::encode_bdt_entries(std::slice::from_ref(&seeded_entry), &mut seed_buf);
    fs::write(persist.path(), &seed_buf).unwrap();

    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![fallback_entry.clone()]);
    bbmd_transport.set_bdt_persist_path(persist.path().to_path_buf());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (_bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    // Startup prefers the seeded file over the configured fallback.
    let startup_bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        startup_bdt.iter().any(|entry| entry == &seeded_entry),
        "restart must load the seeded BDT entry"
    );
    assert!(
        !startup_bdt.iter().any(|entry| entry == &fallback_entry),
        "restart must prefer valid persisted BDT data over configured fallback"
    );
    let file_before = fs::read(persist.path()).unwrap();

    // Valid inbound Write-BDT NAKs and mutates neither memory nor disk.
    let replacement = BdtEntry {
        ip: [203, 0, 113, 7],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    assert_eq!(
        client_transport
            .write_bdt(&bbmd_mac, std::slice::from_ref(&replacement))
            .await
            .unwrap_err()
            .nak(),
        Some(BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK)
    );
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|entry| entry == &seeded_entry),
        "rejected Write-BDT must preserve seeded BDT memory"
    );
    assert!(
        !bdt.iter().any(|entry| entry == &replacement),
        "rejected Write-BDT must not apply the replacement entry"
    );
    assert_eq!(
        fs::read(persist.path()).unwrap(),
        file_before,
        "rejected valid Write-BDT must not change the persisted file"
    );

    // Malformed inbound Write-BDT NAKs and likewise changes nothing.
    let response = raw_bvlc_request(
        &bbmd_mac,
        BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
        &[0; bacnet_bip_codec::BDT_ENTRY_LENGTH - 1],
    )
    .await;
    assert_eq!(
        decode_bvlc_result_code(&response).unwrap(),
        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
    );
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|entry| entry == &seeded_entry),
        "malformed Write-BDT must preserve seeded BDT memory"
    );
    assert_eq!(
        fs::read(persist.path()).unwrap(),
        file_before,
        "malformed Write-BDT must not change the persisted file"
    );

    bbmd_transport.stop().await.unwrap();

    // Restart still loads the seeded file, proving the rejected writes did not
    // mutate the persisted state.
    let mut restarted_bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, bbmd_port, Ipv4Addr::BROADCAST);
    restarted_bbmd.enable_bbmd(vec![fallback_entry.clone()]);
    restarted_bbmd.set_bdt_persist_path(persist.path().to_path_buf());
    let _restarted_rx = restarted_bbmd.start().await.unwrap();
    assert_eq!(restarted_bbmd.local_mac(), bbmd_mac.as_slice());

    let restarted_bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        restarted_bdt.iter().any(|entry| entry == &seeded_entry),
        "restart must still load the seeded BDT entry"
    );
    assert!(
        !restarted_bdt.iter().any(|entry| entry == &replacement),
        "restart must not contain the rejected replacement entry"
    );

    client_transport.stop().await.unwrap();
    restarted_bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_persisted_bdt_falls_back_to_configured_bdt() {
    let persist = TempBdtFile::new("bdt-invalid");
    fs::write(persist.path(), [0x81, 0x01, 0x00]).unwrap();

    let fallback_entry = BdtEntry {
        ip: [198, 51, 100, 12],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![fallback_entry.clone()]);
    bbmd_transport.set_bdt_persist_path(persist.path().to_path_buf());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|entry| entry == &fallback_entry),
        "invalid persisted bytes must not replace the configured BDT"
    );

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn semantically_invalid_persisted_bdt_falls_back_to_configured_bdt() {
    let persist = TempBdtFile::new("bdt-semantic-invalid");
    // Structurally valid wire format (one 10-byte entry) but semantically
    // invalid: UDP port 0 must be rejected before commit.
    let invalid_entry = BdtEntry {
        ip: [192, 0, 2, 44],
        port: 0,
        broadcast_mask: [255, 255, 255, 255],
    };
    let mut seed_buf = BytesMut::new();
    bbmd::encode_bdt_entries(std::slice::from_ref(&invalid_entry), &mut seed_buf);
    fs::write(persist.path(), &seed_buf).unwrap();

    let fallback_entry = BdtEntry {
        ip: [198, 51, 100, 12],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![fallback_entry.clone()]);
    bbmd_transport.set_bdt_persist_path(persist.path().to_path_buf());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|entry| entry == &fallback_entry),
        "semantically invalid persisted BDT must fall back to the configured BDT"
    );
    assert!(
        !bdt.iter().any(|entry| entry == &invalid_entry),
        "semantically invalid persisted entry must not be committed"
    );

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn conflicting_persisted_bdt_falls_back_to_configured_bdt() {
    let persist = TempBdtFile::new("bdt-conflict");
    // Same (ip, port) with different masks conflicts and must be rejected.
    let first = BdtEntry {
        ip: [192, 0, 2, 45],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    };
    let conflicting = BdtEntry {
        ip: [192, 0, 2, 45],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let mut seed_buf = BytesMut::new();
    bbmd::encode_bdt_entries(&[first.clone(), conflicting.clone()], &mut seed_buf);
    fs::write(persist.path(), &seed_buf).unwrap();

    let fallback_entry = BdtEntry {
        ip: [198, 51, 100, 13],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![fallback_entry.clone()]);
    bbmd_transport.set_bdt_persist_path(persist.path().to_path_buf());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|entry| entry == &fallback_entry),
        "conflicting persisted BDT must fall back to the configured BDT"
    );
    assert!(
        !bdt.iter().any(|entry| entry == &first),
        "conflicting persisted entries must not be committed"
    );

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_configured_bdt_fails_startup() {
    let invalid_entry = BdtEntry {
        ip: [224, 0, 0, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    };
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![invalid_entry]);
    let err = bbmd_transport.start().await.unwrap_err();
    let text = format!("{err}");
    assert!(
        text.contains("BDT configuration error"),
        "invalid configured BDT must fail startup, got: {text}"
    );
    assert!(
        text.to_lowercase().contains("multicast"),
        "invalid configured error must name the category, got: {text}"
    );
}
