use super::super::application::Application;
use super::super::config::PortConfig;
use super::*;
use bacnet_transport::bip::BipMode;
use tokio::sync::oneshot;

async fn started() -> (Lifecycle<NativeSession>, u64) {
    let owner = Lifecycle::new();
    let generation = owner
        .start(false, || {
            NativeSession::new(
                vec![PortConfig {
                    network_number: 1,
                    interface: std::net::Ipv4Addr::LOCALHOST,
                    port: 0,
                    broadcast: std::net::Ipv4Addr::LOCALHOST,
                    mode: BipMode::Normal,
                }],
                Application::None,
            )
        })
        .await
        .unwrap()
        .publish(|_| {});
    (owner, generation)
}

#[tokio::test]
async fn lost_publication_at_each_bridge_boundary_joins_its_generation() {
    Python::initialize();
    for via in [Via::BeforeDelivery, Via::OnLoop, Via::ClosedLoop] {
        let (owner, generation) = started().await;
        let publication = Publication::new(owner.clone());
        publication.arm(generation);
        let (tx, done) = oneshot::channel();
        *publication.completed.lock().unwrap() = Some(tx);
        Python::attach(|py| publication.put_back(py, py.None(), via));
        publication.settled();
        done.await.unwrap();
        assert!(owner.inspect(|_| ()).await.is_none());
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn unsettled_drop_cleans_but_a_delivered_result_keeps_ownership() {
    let (owner, generation) = started().await;
    let publication = Publication::new(owner.clone());
    publication.arm(generation);
    let (tx, done) = oneshot::channel();
    *publication.completed.lock().unwrap() = Some(tx);
    drop(publication);
    done.await.unwrap();
    assert!(owner.inspect(|_| ()).await.is_none());
    let (owner, generation) = started().await;
    let publication = Publication::new(owner.clone());
    publication.arm(generation);
    let (tx, done) = oneshot::channel();
    *publication.completed.lock().unwrap() = Some(tx);
    publication.settled();
    drop(publication);
    assert!(done.await.is_err());
    assert!(owner.inspect(|_| ()).await.is_some());
    owner.close().await.unwrap();
}

#[tokio::test]
async fn unread_start_and_context_entry_clean_before_publication_is_armed() {
    use std::future::Future;
    for reenter in [false, true] {
        let owner = Lifecycle::new();
        let publication = Publication::new(owner.clone());
        let configs = vec![PortConfig {
            network_number: 1,
            interface: std::net::Ipv4Addr::LOCALHOST,
            port: 0,
            broadcast: std::net::Ipv4Addr::LOCALHOST,
            mode: BipMode::Normal,
        }];
        let mut start = Box::pin(publication.start(reenter, configs, Application::None));
        std::future::poll_fn(|cx| {
            assert!(start.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let ports = owner.inspect(NativeSession::ports).await.unwrap();
        let mac = &ports[0].mac;
        let address = std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]),
            u16::from_be_bytes([mac[4], mac[5]]),
        );
        assert_eq!(publication.generation.load(Ordering::SeqCst), 0);
        let done = owner.observe_cleanup_for_test().await;
        drop(start);
        // Keep the unarmed Publication alive: only the unread receipt can
        // initiate cleanup. Completion is observed after native owner drop.
        done.await.unwrap();
        assert!(owner.inspect(|_| ()).await.is_none());
        let rebound = std::net::UdpSocket::bind(address).unwrap();
        drop(rebound);
        owner.close().await.unwrap();
        drop(publication);
    }
}

#[tokio::test]
async fn successful_receipt_transfer_arms_the_guard_and_keeps_delivered_owner() {
    for reenter in [false, true] {
        let owner = Lifecycle::new();
        let publication = Publication::new(owner.clone());
        publication
            .start(
                reenter,
                vec![PortConfig {
                    network_number: 1,
                    interface: std::net::Ipv4Addr::LOCALHOST,
                    port: 0,
                    broadcast: std::net::Ipv4Addr::LOCALHOST,
                    mode: BipMode::Normal,
                }],
                Application::None,
            )
            .await
            .unwrap();
        assert_ne!(publication.generation.load(Ordering::SeqCst), 0);
        publication.settled();
        drop(publication);
        assert!(owner.inspect(|_| ()).await.is_some());
        owner.close().await.unwrap();
    }
}
