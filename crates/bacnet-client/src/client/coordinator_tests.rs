use std::collections::HashSet;
use std::sync::Arc;

use bacnet_encoding::apdu::{self, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[1];
const ROUTER_MAC: &[u8] = &[2];

async fn receive_invoke_id(
    receiver: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) -> u8 {
    let received = timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("request send timed out")
        .expect("request channel closed");
    let npdu = decode_npdu(received.npdu).expect("valid request NPDU");
    match apdu::decode_apdu(npdu.payload).expect("valid request APDU") {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    }
}

async fn wait_for_active_count(client: &BACnetClient<LoopbackTransport>, expected: usize) {
    timeout(Duration::from_secs(2), async {
        loop {
            if client.tsm.lock().await.coordinated_active_count() == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("coordinator active count did not reach {expected}"));
}

async fn seed_pending_transaction(client: &BACnetClient<LoopbackTransport>) {
    client
        .tsm
        .lock()
        .await
        .register_coordinated_transaction_with_progress(
            MacAddr::from_slice(ROUTER_MAC),
            CanonicalPeer::direct(ROUTER_MAC),
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
        )
        .unwrap();
}

#[tokio::test]
async fn client_stop_releases_coordinated_pending_transactions() {
    let (client_transport, _peer_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), ROUTER_MAC.to_vec());
    let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
        .await
        .unwrap();
    seed_pending_transaction(&client).await;
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 1);

    client.stop().await.unwrap();
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
}

#[tokio::test]
async fn client_drop_releases_coordinated_pending_transactions() {
    let (client_transport, _peer_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), ROUTER_MAC.to_vec());
    let client = BACnetClient::start(ClientConfig::default(), client_transport)
        .await
        .unwrap();
    seed_pending_transaction(&client).await;
    let tsm = Arc::clone(&client.tsm);
    assert_eq!(tsm.lock().await.coordinated_active_count(), 1);

    drop(client);
    assert_eq!(tsm.lock().await.coordinated_active_count(), 0);
}

#[tokio::test]
async fn initial_send_failure_cancels_the_exact_coordinated_lease() {
    let (client_transport, peer_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), ROUTER_MAC.to_vec());
    drop(peer_transport);
    let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
        .await
        .unwrap();

    assert!(client
        .confirmed_request(ROUTER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C],)
        .await
        .is_err());
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);

    client.stop().await.unwrap();
}

#[tokio::test]
async fn client_uses_one_global_generation_safe_pool_across_direct_and_routed_peers() {
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), ROUTER_MAC.to_vec());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(
            ClientConfig {
                apdu_timeout_ms: 60_000,
                apdu_retries: 0,
                ..ClientConfig::default()
            },
            client_transport,
        )
        .await
        .unwrap(),
    );
    let mut requests: Vec<JoinHandle<Result<Bytes, Error>>> = Vec::new();
    let mut invoke_ids = HashSet::new();

    for index in 0..256u16 {
        let request_client = Arc::clone(&client);
        let request = if index % 2 == 0 {
            let destination = index.to_be_bytes().to_vec();
            tokio::spawn(async move {
                request_client
                    .confirmed_request(&destination, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
                    .await
            })
        } else {
            let destination = index.to_be_bytes().to_vec();
            tokio::spawn(async move {
                request_client
                    .confirmed_request_routed(
                        ROUTER_MAC,
                        index,
                        &destination,
                        ConfirmedServiceChoice::READ_PROPERTY,
                        &[0x0C],
                    )
                    .await
            })
        };
        let invoke_id = receive_invoke_id(&mut peer_rx).await;
        assert!(invoke_ids.insert(invoke_id));
        requests.push(request);
    }
    assert_eq!(invoke_ids.len(), 256);
    wait_for_active_count(&client, 256).await;

    let exhausted = timeout(
        Duration::from_secs(1),
        client.confirmed_request(&[0xFF], ConfirmedServiceChoice::READ_PROPERTY, &[0x0C]),
    )
    .await
    .expect("exhaustion did not return synchronously");
    assert!(matches!(
        exhausted,
        Err(Error::Encoding(message)) if message == "all invoke IDs exhausted for destination"
    ));
    assert!(timeout(Duration::from_millis(50), peer_rx.recv())
        .await
        .is_err());

    let released_invoke_id = 0;
    let first = requests.remove(0);
    first.abort();
    let _ = first.await;
    wait_for_active_count(&client, 255).await;

    let replacement_client = Arc::clone(&client);
    let replacement = tokio::spawn(async move {
        replacement_client
            .confirmed_request(&[0, 0], ConfirmedServiceChoice::READ_PROPERTY, &[0x0D])
            .await
    });
    assert_eq!(receive_invoke_id(&mut peer_rx).await, released_invoke_id);
    wait_for_active_count(&client, 256).await;

    replacement.abort();
    let _ = replacement.await;
    for request in requests {
        request.abort();
        let _ = request.await;
    }
    wait_for_active_count(&client, 0).await;

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still shared"));
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}
