use super::*;
use bacnet_transport::bip6::{Bip6ForeignDeviceConfig, Bip6Vmac};

async fn foreign_source(automatic: bool) {
    let (selected, index) = fixture();
    let bbmd = udp(selected, 0, index, false);
    let bbmd_port = bbmd.local_addr().unwrap().port();
    let requested = if automatic {
        Ipv6Addr::UNSPECIFIED
    } else {
        selected
    };
    let mut transport = Bip6Transport::new(requested, 0, Some(0x12_3456));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: selected,
        bbmd_port,
        ttl: 60,
    });
    let mut incoming = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let (announced, port) = (*local.ip(), local.port());
    let registration = tokio::time::timeout(DEADLINE, wire::receive(&bbmd))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        registration.bytes,
        [0x82, 0x09, 0, 9, 0x12, 0x34, 0x56, 0, 60]
    );
    transport.send_broadcast(NPDU).await.unwrap();
    let dbtn = tokio::time::timeout(DEADLINE, wire::receive(&bbmd))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        dbtn.bytes,
        [0x82, 0x0c, 0, 11, 0x12, 0x34, 0x56, 1, 0, 0x10, 8]
    );
    let mut forwarded = vec![0x82, 0x08, 0, 29, 0x40, 0x88, 0x71];
    forwarded.extend_from_slice(&selected.octets());
    forwarded.extend_from_slice(&bbmd_port.to_be_bytes());
    forwarded.extend_from_slice(NPDU);
    bbmd.send_to(&forwarded, registration.source).await.unwrap();
    let received = tokio::time::timeout(DEADLINE, incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), NPDU);
    assert!(received.link_layer_group);
    // The origin's VMAC is the source MAC; its carried address is learned.
    assert_eq!(received.source_mac.as_slice(), [0x40, 0x88, 0x71]);
    assert_eq!(
        transport
            .lookup_vmac(Bip6Vmac::new([0x40, 0x88, 0x71]))
            .map(|origin| (*origin.ip(), origin.port())),
        Some((selected, bbmd_port))
    );
    transport
        .send_unicast(NPDU, &received.source_mac)
        .await
        .unwrap();
    let unicast = tokio::time::timeout(DEADLINE, wire::receive(&bbmd))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unicast.bytes,
        [
            0x82, 1, 0, 14, 0x12, 0x34, 0x56, 0x40, 0x88, 0x71, 1, 0, 0x10, 8
        ]
    );
    // A byte-identical Forwarded-NPDU from an unconfigured port is rejected.
    // The later trusted marker establishes receive progress without sleeps.
    let impostor = udp(selected, 0, index, false);
    impostor
        .send_to(&forwarded, registration.source)
        .await
        .unwrap();
    let marker = [1, 0, 0x10, 8, 0x09, 17, 0x19, 17];
    forwarded[2..4].copy_from_slice(&33u16.to_be_bytes());
    forwarded.truncate(25);
    forwarded.extend_from_slice(&marker);
    bbmd.send_to(&forwarded, registration.source).await.unwrap();
    let admitted = tokio::time::timeout(DEADLINE, incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admitted.npdu.as_ref(), marker);
    eprintln!(
        "foreign requested={requested} announced=[{announced}]:{port} registration={registration:?} dbtn={dbtn:?}"
    );
    transport.stop().await.unwrap();
    assert_eq!(
        announced, selected,
        "foreign advertised identity must be a real BBMD-usable local source"
    );
    assert!(incoming.recv().await.is_none());
    assert_eq!(transport.local_mac(), &[0; 3]);
    for frame in [registration, dbtn, unicast] {
        assert_eq!((*frame.source.ip(), frame.source.port()), (announced, port));
        assert_eq!(frame.destination, selected);
    }
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 link and raw BBMD"]
async fn automatic_foreign_identity_matches_registration_and_dbtn_source() {
    foreign_source(true).await;
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 link and raw BBMD"]
async fn explicit_foreign_identity_matches_registration_and_dbtn_source() {
    foreign_source(false).await;
}
