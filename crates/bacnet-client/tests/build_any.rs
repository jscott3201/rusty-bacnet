//! Public builder return types do not require a serial-port instance.
use bacnet_client::client::BACnetClient;
use bacnet_transport::any::AnyTransport;

#[tokio::test]
async fn bip_build_any_returns_the_selected_transport() {
    let mut client: BACnetClient<AnyTransport> = BACnetClient::bip_builder()
        .interface(std::net::Ipv4Addr::LOCALHOST)
        .port(0)
        .build_any()
        .await
        .unwrap();
    assert!(matches!(client.transport(), AnyTransport::Bip(_)));
    client.stop().await.unwrap();
}

#[cfg(feature = "ipv6")]
#[tokio::test]
async fn bip6_build_any_returns_the_selected_transport() {
    let mut client: BACnetClient<AnyTransport> = BACnetClient::bip6_builder()
        .interface(std::net::Ipv6Addr::LOCALHOST)
        .port(0)
        .device_instance(123)
        .build_any()
        .await
        .unwrap();
    let AnyTransport::Bip6(transport) = client.transport() else {
        panic!("expected B/IPv6 transport");
    };
    assert_eq!(
        transport.local_vmac(),
        Some(bacnet_transport::bip6::Bip6Vmac::new([0, 0, 123]))
    );
    assert_eq!(
        *transport.local_address().unwrap().ip(),
        std::net::Ipv6Addr::LOCALHOST
    );
    client.stop().await.unwrap();
}

#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn sc_build_any_checks_identity_before_tls() {
    let result: Result<BACnetClient<AnyTransport>, _> =
        BACnetClient::sc_builder().build_any().await;
    assert!(result.err().unwrap().to_string().contains("unknown VMAC"));
}
