//! ACK requires real accepting capability, independently of URI knowledge.
use super::*;
use crate::sc_tls::DirectListener;
use tokio::time::timeout;

async fn started(
    uris: &[&str],
) -> (
    ScTransport<LoopbackWebSocket>,
    DirectListener,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (client, hub) = LoopbackWebSocket::pair();
    let transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(uris.to_vec());
    let (mut transport, listener) = listener::register_listener(transport, [1; 6], [1; 16]).await;
    let (rx, ()) = tokio::join!(
        transport.start(),
        data_attribute_tests::hub_accept(&hub, [0x10; 6])
    );
    (transport, listener, rx.unwrap(), hub)
}

async fn answer(hub: &LoopbackWebSocket, id: u16, source: Option<Vmac>, uris: Option<&[&str]>) {
    let [hi, lo] = id.to_be_bytes();
    let mut request = vec![2, if source.is_some() { 8 } else { 0 }, hi, lo];
    if let Some(vmac) = source {
        request.extend(vmac);
    }
    hub.send(&request).await.unwrap();
    let mut expected = vec![
        if uris.is_some() { 3 } else { 0 },
        if source.is_some() { 4 } else { 0 },
        hi,
        lo,
    ];
    if let Some(vmac) = source {
        expected.extend(vmac);
    }
    match uris {
        Some(uris) => expected.extend(uris.join(" ").as_bytes()),
        None => expected.extend([2, 1, 0, 0, 7, 0, 45]),
    }
    assert_eq!(
        timeout(Duration::from_secs(2), hub.recv())
            .await
            .unwrap()
            .unwrap(),
        expected
    );
    // Strict ordering: a duplicate/unexpected reply cannot be drained away.
    hub.send(&[10, 0, 0x44, 0x55, 0x42]).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(2), hub.recv())
            .await
            .unwrap()
            .unwrap(),
        [0, 0, 0x44, 0x55, 10, 1, 0, 0, 7, 0, 7]
    );
}

#[tokio::test]
async fn live_listener_ack_preserves_empty_and_known_uris_ids_and_addresses() {
    for uris in [
        vec![],
        vec!["wss://one.example/sc", "wss://two.example:8443/sc"],
    ] {
        let (mut transport, mut listener, mut rx, hub) = started(&uris).await;
        for id in [0, u16::MAX] {
            for source in [None, Some([0x22; 6])] {
                answer(&hub, id, source, Some(&uris)).await;
            }
        }
        assert!(rx.try_recv().is_err());
        listener.stop().await;
        transport.stop().await.unwrap();
    }
}

#[tokio::test]
async fn capability_tracks_identity_application_intake_and_listener_lifecycle() {
    for close in ["stop", "drop", "intake"] {
        let uris = ["wss://one.example/sc"];
        let (mut transport, listener, rx, hub) = started(&uris).await;
        let mut listener = Some(listener);
        let mut rx = Some(rx);
        answer(&hub, 1, None, Some(&uris)).await;
        let conn = transport.connection().unwrap();
        conn.lock().await.local_vmac = [9; 6];
        answer(&hub, 2, Some([0x22; 6]), None).await;
        conn.lock().await.local_vmac = [1; 6];
        conn.lock().await.device_uuid = [9; 16];
        answer(&hub, 3, None, None).await;
        conn.lock().await.device_uuid = [1; 16];
        answer(&hub, 4, None, Some(&uris)).await;
        match close {
            "stop" => listener.as_mut().unwrap().stop().await,
            "drop" => drop(listener.take()),
            "intake" => drop(rx.take()),
            _ => unreachable!(),
        }
        answer(&hub, 5, Some([0x22; 6]), None).await;
        if let Some(listener) = &mut listener {
            listener.stop().await;
        }
        transport.stop().await.unwrap();
    }
}
