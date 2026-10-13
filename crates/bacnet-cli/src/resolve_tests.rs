use std::net::Ipv6Addr;

use bacnet_transport::bip::BipTransport;

use super::*;

fn mac(target: Result<Target, String>) -> Vec<u8> {
    match target {
        Ok(Target::Mac(mac)) => mac,
        Ok(_) => panic!("expected a MAC target"),
        Err(e) => panic!("expected a MAC target, got {e}"),
    }
}

fn bip6_address(target: Result<Target, String>) -> SocketAddrV6 {
    match target {
        Ok(Target::Bip6(peer)) => peer,
        Ok(_) => panic!("expected a B/IPv6 address target"),
        Err(e) => panic!("expected a B/IPv6 address target, got {e}"),
    }
}

#[test]
fn test_ip_to_bip_mac() {
    let mac = ip_to_bip_mac(Ipv4Addr::new(192, 168, 1, 100), 47808);
    assert_eq!(mac, vec![192, 168, 1, 100, 0xBA, 0xC0]);
}

#[test]
fn test_parse_target_ip_no_port() {
    assert_eq!(
        mac(parse_target("192.168.1.100")),
        vec![192, 168, 1, 100, 0xBA, 0xC0]
    );
}

#[test]
fn test_parse_target_ip_with_port() {
    assert_eq!(
        mac(parse_target("10.0.1.100:47809")),
        vec![10, 0, 1, 100, 0xBA, 0xC1]
    );
}

#[test]
fn test_parse_target_instance() {
    let target = parse_target("1234").unwrap();
    match target {
        Target::Instance(id) => assert_eq!(id, 1234),
        _ => panic!("expected Instance target"),
    }
}

#[test]
fn test_parse_target_routed() {
    let target = parse_target("2:1234").unwrap();
    match target {
        Target::Routed(dnet, id) => {
            assert_eq!(dnet, 2);
            assert_eq!(id, 1234);
        }
        _ => panic!("expected Routed target"),
    }
}

#[test]
fn test_parse_target_invalid() {
    assert!(parse_target("not-a-target").is_err());
}

/// A B/IPv6 node is named by its VMAC (Clause U.1.3): a device instance's
/// 22 bits, high octet first (Clause H.7.2), or any three octets.
#[test]
fn bip6_targets_name_a_vmac() {
    assert_eq!(mac(parse_target("bip6:1234")), [0x00, 0x04, 0xD2]);
    assert_eq!(mac(parse_target("bip6:0")), [0, 0, 0]);
    assert_eq!(mac(parse_target("bip6:4194303")), [0x3F, 0xFF, 0xFF]);
    assert_eq!(mac(parse_target("bip6:0x40ABCD")), [0x40, 0xAB, 0xCD]);
    assert_eq!(mac(parse_target("bip6:0x40abcd")), [0x40, 0xAB, 0xCD]);
    assert_eq!(mac(parse_target("bip6:0X40ABCD")), [0x40, 0xAB, 0xCD]);
}

/// What the CLI prints, it reads back.
#[test]
fn bip6_vmac_text_round_trips() {
    for vmac in [[0x00, 0x04, 0xD2], [0x40, 0xAB, 0xCD], [0x3F, 0xFF, 0xFF]] {
        let printed = crate::output::format_mac(&vmac);
        assert_eq!(mac(parse_target(&printed)), vmac, "{printed}");
    }
    assert_eq!(crate::output::format_mac(&[0, 4, 0xD2]), "bip6:0x0004D2");
}

#[test]
fn bip6_targets_refuse_bad_forms() {
    for target in [
        "bip6:4194304",
        "bip6:4294967295",
        "bip6:99999999999999999999",
    ] {
        let error = parse_target(target).err().unwrap();
        assert!(
            error.contains("must be in 0..=4194303"),
            "{target}: {error}"
        );
    }
    for target in [
        "bip6:",
        "bip6:-1",
        "bip6:12a",
        "bip6:0x",
        "bip6:0x0004D",
        "bip6:0x0004D2F",
        "bip6:0xGG0000",
    ] {
        let error = parse_target(target).err().unwrap();
        assert!(error.contains("bip6:0x<6 hex digits>"), "{target}: {error}");
    }
}

#[test]
fn bracketed_ipv6_is_an_address_to_resolve() {
    let peer = bip6_address(parse_target("[::1]:47808"));
    assert_eq!((*peer.ip(), peer.port()), (Ipv6Addr::LOCALHOST, 47808));
    let peer = bip6_address(parse_target("[fe80::1]"));
    assert_eq!(peer.ip().segments()[0], 0xfe80);
    assert_eq!(peer.port(), 0xBAC0, "the default port");
    let peer = bip6_address(parse_target("[fe80::1%3]:47809"));
    assert_eq!((peer.scope_id(), peer.port()), (3, 47809));
    for target in ["[::1", "[::1]:x", "[nope]:47808", "[::1]47808"] {
        let error = parse_target(target).err().unwrap();
        assert!(error.contains("[IPv6]:port"), "{target}: {error}");
    }
}

/// `[ip]:port` becomes the VMAC of the node there, asked once with a
/// unicast Virtual-Address-Resolution (Clause U.5). Both ends have a device
/// instance: a random VMAC would need a multicast probe at start, which
/// Linux loopback can't carry.
#[tokio::test]
async fn a_bip6_address_resolves_to_the_vmac_of_the_node_there() {
    let mut server = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1234));
    let mut client = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(4321));
    let _server_rx = server.start().await.unwrap();
    let _client_rx = client.start().await.unwrap();
    let server_address = server.local_address().unwrap();

    let vmac = bip6_vmac(&client, server_address).await.unwrap();
    assert_eq!(vmac, [0x00, 0x04, 0xD2]);
    let text = format!("[::1]:{}", server_address.port());
    assert_eq!(
        address_mac(&client, &text, "unused").await.unwrap(),
        [0x00, 0x04, 0xD2]
    );
    assert_eq!(
        address_mac(&client, "bip6:1234", "unused").await.unwrap(),
        [0x00, 0x04, 0xD2]
    );
    assert_eq!(
        address_mac(&client, "1234", "needs an address").await,
        Err("needs an address".to_string())
    );
    assert_eq!(
        address_mac(&client, "2:1234", "needs an address").await,
        Err("needs an address".to_string())
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_bip6_address_needs_the_bip6_transport() {
    let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let peer = "[::1]:47808".parse().unwrap();
    let error = bip6_vmac(&bip, peer).await.unwrap_err();
    assert!(error.contains("--ipv6"), "{error}");
    assert!(as_bip6(&bip).is_none());
}

/// A `bip6:` VMAC names a B/IPv6 node, so a command on another transport
/// refuses it up front, rather than hand B/IP a 3-octet MAC.
#[tokio::test]
async fn a_bip6_vmac_needs_the_bip6_transport() {
    let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    for target in ["bip6:1234", "bip6:0x40ABCD"] {
        let error = parse_target_on(&bip, target).err().unwrap();
        assert!(
            error.contains("bip6: addresses need the --ipv6 transport"),
            "{error}"
        );
        let error = address_mac(&bip, target, "unused").await.unwrap_err();
        assert!(error.contains("--ipv6"), "{error}");
    }
    // Other forms parse on B/IP as before.
    assert_eq!(
        mac(parse_target_on(&bip, "10.0.0.1")),
        [10, 0, 0, 1, 0xBA, 0xC0]
    );
}
