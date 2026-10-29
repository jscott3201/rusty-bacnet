//! A built client lends its transport, and the BBMD helpers reach B/IP
//! through `AnyTransport` (#956).

use super::*;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::{AsBip, BvlcExchangeError, ManagementCounters};
use bacnet_transport::bvll::BvlcResultCode;
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::data_link::DataLink;

/// A remote BBMD row the in-process BBMD starts with.
const PEER_ROW: BdtEntry = BdtEntry {
    ip: [10, 10, 0, 9],
    port: 0xBAC0,
    broadcast_mask: [255, 255, 255, 0],
};

/// A client whose own transport is a loopback BBMD that admits foreign
/// devices and lets 127.0.0.1 delete FDT entries.
async fn bbmd_client() -> BACnetClient<BipTransport> {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(vec![PEER_ROW]);
    transport.set_bbmd_management_acl(vec![[127, 0, 0, 1]]);
    transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    BACnetClient::generic_builder()
        .transport(transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn bip_client_transport_counters_change_after_a_read_bdt_exchange() {
    let mut bbmd = bbmd_client().await;
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    // The borrow is the live transport the client was built with.
    assert_eq!(client.transport().local_mac(), client.local_mac());
    // bip_builder makes a plain B/IP node: no BBMD state or FDT.
    assert!(client.transport().bbmd_state().is_none());
    assert!(client.transport().fdt_counters().is_none());
    assert_eq!(
        bbmd.transport().management_counters(),
        ManagementCounters::default()
    );

    let bdt = client.read_bdt(bbmd.local_mac()).await.unwrap();
    assert!(bdt.contains(&PEER_ROW), "BDT {bdt:?} lacks the peer row");

    // The BBMD records its ACK before sending it, so the count is in place
    // once the requester has the table.
    let after = bbmd.transport().management_counters();
    assert_eq!(after.read_bdt_responses, 1);
    assert_eq!(after.response_bytes_sent, 4 + 10 * bdt.len() as u64);
    // Management counters count ACKs this transport sent; the requester sent none.
    assert_eq!(
        client.transport().management_counters(),
        ManagementCounters::default()
    );
    assert_eq!(client.transport().fanout_counters().packets_forwarded, 0);

    // The BBMD state handle is an owned Arc: it outlives the borrow.
    let state = Arc::clone(bbmd.transport().bbmd_state().expect("BBMD mode"));
    assert_eq!(state.lock().unwrap().bdt(), bdt.as_slice());
    assert!(bbmd.transport().fdt_counters().is_some());

    client.stop().await.unwrap();
    bbmd.stop().await.unwrap();
    // The owned handle still reads the last tables after the client has
    // stopped its transport, as an MS/TP diagnostics handle does.
    assert_eq!(state.lock().unwrap().bdt(), bdt.as_slice());
}

#[tokio::test]
async fn any_transport_bip_variant_runs_the_bbmd_helpers_against_an_in_process_bbmd() {
    let mut bbmd = bbmd_client().await;
    let bbmd_mac = bbmd.local_mac().to_vec();
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut client = BACnetClient::generic_builder()
        .transport(AnyTransport::from(transport))
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let bdt = client.read_bdt(&bbmd_mac).await.unwrap();
    assert!(bdt.contains(&PEER_ROW), "BDT {bdt:?} lacks the peer row");
    assert_eq!(
        client.write_bdt(&bbmd_mac, &[]).await.unwrap_err().nak(),
        Some(BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK)
    );

    client
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    let fdt_counters = bbmd.transport().fdt_counters().expect("BBMD mode");
    assert_eq!(fdt_counters.registrations_accepted, 1);
    let (ip, port) = decode_bip_mac(client.local_mac()).unwrap();
    let fdt = client.read_fdt(&bbmd_mac).await.unwrap();
    assert!(
        fdt.iter()
            .any(|entry| entry.ip == ip && entry.port == port && entry.ttl == 60),
        "FDT {fdt:?} lacks this client"
    );
    client.delete_fdt_entry(&bbmd_mac, ip, port).await.unwrap();
    assert!(client.read_fdt(&bbmd_mac).await.unwrap().is_empty());

    // The B/IP counters are reachable through the same view.
    let counters = client.transport().as_bip().unwrap().management_counters();
    assert_eq!(counters, ManagementCounters::default());
    assert_eq!(bbmd.transport().management_counters().read_bdt_responses, 1);
    assert_eq!(bbmd.transport().management_counters().read_fdt_responses, 2);

    client.stop().await.unwrap();
    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn bbmd_helpers_on_a_non_bip_any_transport_fail_typed_before_sending() {
    let (ours, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut peer_rx = peer.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(AnyTransport::Loopback(ours))
        .build()
        .await
        .unwrap();
    let target = [127, 0, 0, 1, 0xBA, 0xC0];

    fn refused<V, E: Into<BvlcExchangeError>>(result: Result<V, E>) -> bool {
        matches!(
            result.map_err(Into::into),
            Err(BvlcExchangeError::Transport(Error::UnsupportedTransport {
                required: DataLink::Bip,
                actual: DataLink::Loopback,
            }))
        )
    }
    assert!(refused(client.read_bdt(&target).await));
    assert!(refused(client.write_bdt(&target, &[PEER_ROW]).await));
    assert!(refused(client.read_fdt(&target).await));
    assert!(refused(
        client
            .delete_fdt_entry(&target, [127, 0, 0, 1], 0xBAC0)
            .await
    ));
    assert!(refused(
        client.register_foreign_device_bvlc(&target, 60).await
    ));
    assert!(refused(client.transport().as_bip()));
    assert!(
        peer_rx.try_recv().is_err(),
        "a refused helper must not send anything"
    );

    client.stop().await.unwrap();
}

/// A B/IPv6 client built with a foreign registration registers with its
/// BBMD at start (#1941).
#[cfg(feature = "ipv6")]
#[tokio::test]
async fn a_bip6_client_with_a_foreign_registration_registers_at_start() {
    use bacnet_transport::bip6::Bip6ForeignDeviceConfig;
    let bbmd = tokio::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut client = BACnetClient::bip6_builder()
        .interface(Ipv6Addr::LOCALHOST)
        .port(0)
        .device_instance(0x12_3456)
        .foreign_device(Bip6ForeignDeviceConfig {
            bbmd_ip: Ipv6Addr::LOCALHOST,
            bbmd_port: bbmd.local_addr().unwrap().port(),
            ttl: 90,
        })
        .build_any()
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let (len, _) = tokio::time::timeout(Duration::from_secs(5), bbmd.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    // Register-Foreign-Device from the instance's VMAC, for 90 s.
    assert_eq!(&buf[..len], [0x82, 0x09, 0, 9, 0x12, 0x34, 0x56, 0, 90]);
    client.stop().await.unwrap();
}
