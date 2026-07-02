use super::*;
use tokio::time::{timeout, Duration};

async fn wait_for_fdt_len(transport: &BipTransport, expected: usize) {
    let state = transport.bbmd_state().unwrap().clone();
    timeout(Duration::from_secs(1), async {
        loop {
            let len = {
                let state = state.lock().await;
                state.fdt_len_for_test()
            };
            if len == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for FDT length {expected}"));
}

#[tokio::test]
async fn bbmd_fdt_purge_task_clears_expired_entry_without_bvlc_request() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    assert!(bbmd_transport.bbmd_fdt_purge_task.is_some());
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let (fd_ip, fd_port) = decode_bip_mac(client_transport.local_mac()).unwrap();
    wait_for_fdt_len(&bbmd_transport, 1).await;

    {
        let state = bbmd_transport.bbmd_state().unwrap();
        let mut state = state.lock().await;
        state.backdate_foreign_device_for_test(fd_ip, fd_port, Duration::from_secs(91));
        assert_eq!(state.fdt_len_for_test(), 1);
    }

    wait_for_fdt_len(&bbmd_transport, 0).await;

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
    assert!(bbmd_transport.bbmd_fdt_purge_task.is_none());
}

#[tokio::test]
async fn register_foreign_device_resets_entry_before_purge_task_removes_it() {
    let fd_ip = [192, 0, 2, 10];
    let fd_port = 0xBAC0;
    let bbmd = std::sync::Arc::new(tokio::sync::Mutex::new(BbmdState::new(
        [127, 0, 0, 1],
        0xBAC0,
    )));
    let purge_task = BipTransport::spawn_bbmd_fdt_purge_task(bbmd.clone());

    {
        let mut state = bbmd.lock().await;
        assert_eq!(
            state.register_foreign_device(fd_ip, fd_port, 1),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
        state.backdate_foreign_device_for_test(fd_ip, fd_port, Duration::from_secs(30));
        assert_eq!(
            state.register_foreign_device(fd_ip, fd_port, 1),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
        assert_eq!(state.fdt_len_for_test(), 1);
    }

    tokio::time::sleep(Duration::from_millis(1_200)).await;

    {
        let state = bbmd.lock().await;
        assert_eq!(state.fdt_len_for_test(), 1);
    }

    purge_task.abort();
    let _ = purge_task.await;
}
