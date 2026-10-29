//! The router's application ports as the binding parks and lends them
//! (#2071), on loopback links.
use super::*;
use crate::app_transport::AppTransport;
use bacnet_network::router::{RouterOptions, RouterPort};
use bacnet_transport::bip::AsBip;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

/// A running router on networks 1 and 2, with the far end of each link.
async fn started() -> (StartedRouter, Vec<LoopbackTransport>) {
    let (one, far_one) = LoopbackTransport::pair(vec![0x11], vec![0x21]);
    let (two, far_two) = LoopbackTransport::pair(vec![0x12, 0x02], vec![0x22]);
    let started = BACnetRouter::start(
        vec![
            RouterPort {
                transport: bacnet_transport::any::AnyTransport::Loopback(one),
                network_number: 1,
            },
            RouterPort {
                transport: bacnet_transport::any::AnyTransport::Loopback(two),
                network_number: 2,
            },
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    (started, vec![far_one, far_two])
}

#[test]
fn the_application_keyword() {
    assert_eq!(Application::parse(None).unwrap(), Application::None);
    assert_eq!(
        Application::parse(Some("client")).unwrap(),
        Application::Client
    );
    assert_eq!(
        Application::parse(Some("client_and_server")).unwrap(),
        Application::ClientAndServer
    );
    pyo3::Python::initialize();
    assert!(Application::parse(Some("server")).is_err());
    for application in [
        Application::None,
        Application::Client,
        Application::ClientAndServer,
    ] {
        let name = application.name();
        assert_eq!(Application::parse(name).unwrap(), application);
    }
}

#[tokio::test]
async fn each_share_parks_its_ports_once() {
    for (application, client, server) in [
        (Application::None, false, false),
        (Application::Client, true, false),
        (Application::ClientAndServer, true, true),
    ] {
        let (started, _links) = started().await;
        let (mut router, slots) = Slots::split(application, started);
        assert_eq!(
            slots.take(Role::Client).is_some(),
            client,
            "{application:?}"
        );
        assert_eq!(
            slots.take(Role::Server).is_some(),
            server,
            "{application:?}"
        );
        assert!(slots.take(Role::Client).is_none());
        assert!(slots.take(Role::Server).is_none());
        router.stop().await.unwrap();
    }
}

#[tokio::test]
async fn the_wrapped_port_is_the_router_port_and_has_no_link() {
    let (started, _links) = started().await;
    let (mut router, slots) = Slots::split(Application::Client, started);
    let mut transport = AppTransport::Router(slots.take(Role::Client).unwrap());
    assert!(transport.link().is_none());
    assert!(matches!(transport.as_bip(), Err(Error::Encoding(_))));
    // The longest of the router's MACs is the port's own.
    assert_eq!(transport.local_mac(), [0x12, 0x02]);
    assert!(transport.max_npdu_changes().is_some());
    let _receiver = transport.start().await.unwrap();
    // An NPDU with no DNET is a local send, and the entity has no link.
    let local = [0x01, 0x00, 0x10, 0x08];
    assert!(matches!(
        transport.send_broadcast(&local).await,
        Err(Error::Encoding(message)) if message.contains("no network of its own")
    ));
    router.stop().await.unwrap();
    // Every send after the router stops fails as not connected.
    let global_who_is = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
    match transport.send_broadcast(&global_who_is).await {
        Err(Error::Transport(error)) => {
            assert_eq!(error.kind(), std::io::ErrorKind::NotConnected);
        }
        other => panic!("expected a transport error, got {other:?}"),
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn a_link_passes_through() {
    let (link, _far) = LoopbackTransport::pair(vec![0x31], vec![0x32]);
    let mut transport = AppTransport::from(bacnet_transport::any::AnyTransport::Loopback(link));
    assert!(transport.link().is_some());
    assert_eq!(transport.local_mac(), [0x31]);
    assert!(matches!(
        transport.as_bip(),
        Err(Error::UnsupportedTransport { .. })
    ));
    let _receiver = transport.start().await.unwrap();
    transport.stop().await.unwrap();
}
