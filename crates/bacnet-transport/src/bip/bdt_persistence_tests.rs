use super::*;

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

#[tokio::test]
async fn write_bdt_persists_wire_format_and_restart_loads_it() {
    let persist = TempBdtFile::new("bdt-restart");
    let fallback_entry = BdtEntry {
        ip: [10, 20, 30, 40],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };

    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![fallback_entry.clone()]);
    bbmd_transport.set_bdt_persist_path(persist.path().to_path_buf());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (_bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let persisted_entry = BdtEntry {
        ip: [192, 0, 2, 44],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let result = client_transport
        .write_bdt(&bbmd_mac, std::slice::from_ref(&persisted_entry))
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    let persisted_bytes = fs::read(persist.path()).unwrap();
    let persisted_bdt = BbmdState::decode_bdt(&persisted_bytes).unwrap();
    assert!(
        persisted_bdt.iter().any(|entry| entry == &persisted_entry),
        "persisted BDT must contain the successful Write-BDT entry"
    );
    assert!(
        !persisted_bdt.iter().any(|entry| entry == &fallback_entry),
        "persisted BDT must reflect the replacement table, not the startup fallback"
    );

    bbmd_transport.stop().await.unwrap();

    let mut restarted_bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, bbmd_port, Ipv4Addr::BROADCAST);
    restarted_bbmd.enable_bbmd(vec![fallback_entry.clone()]);
    restarted_bbmd.set_bdt_persist_path(persist.path().to_path_buf());
    let _restarted_rx = restarted_bbmd.start().await.unwrap();
    assert_eq!(restarted_bbmd.local_mac(), bbmd_mac.as_slice());

    let restarted_bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        restarted_bdt.iter().any(|entry| entry == &persisted_entry),
        "restart must load the persisted BDT entry"
    );
    assert!(
        !restarted_bdt.iter().any(|entry| entry == &fallback_entry),
        "restart must prefer valid persisted BDT data over configured fallback"
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
