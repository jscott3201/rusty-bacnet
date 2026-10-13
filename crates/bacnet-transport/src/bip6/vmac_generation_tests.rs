use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bacnet_types::error::Error;
use tokio::net::UdpSocket;

use super::frame::encoded;
use super::vmac_from_device_instance;
use super::{
    format_vmac_text, generate_random_vmac, parse_vmac_text, Bip6ForeignDeviceConfig,
    Bip6Transport, Bip6Vmac, Bvll6, MAX_VMAC_RETRIES,
};
use crate::port::TransportPort;
use crate::port_ownership::lost_port;

#[test]
fn generate_random_vmac_produces_3_bytes() {
    let vmac = generate_random_vmac().unwrap().octets();
    assert_eq!(vmac.len(), 3);
    assert_eq!(vmac[0] & 0xC0, 0x40);
}

/// `bip6:N` is the VMAC device instance N gives, its 22 bits high octet
/// first (H.7.2); `bip6:0x` with six hex digits, in either case, is any
/// VMAC; and the text `format_vmac_text` writes reads back.
#[test]
fn vmac_text_forms() {
    let parse = |text: &str| parse_vmac_text(text).unwrap().octets();
    assert_eq!(parse("bip6:1234"), [0x00, 0x04, 0xD2]);
    assert_eq!(parse("bip6:0"), [0, 0, 0]);
    assert_eq!(parse("bip6:4194303"), [0x3F, 0xFF, 0xFF]);
    assert_eq!(parse("bip6:0x40ABCD"), [0x40, 0xAB, 0xCD]);
    assert_eq!(parse("bip6:0x40abcd"), [0x40, 0xAB, 0xCD]);
    assert_eq!(parse("bip6:0X40ABCD"), [0x40, 0xAB, 0xCD]);
    for octets in [[0x00, 0x04, 0xD2], [0x40, 0xAB, 0xCD], [0xFF, 0x00, 0x01]] {
        let text = format_vmac_text(Bip6Vmac::new(octets));
        assert_eq!(parse(&text), octets, "{text}");
    }
    assert_eq!(
        format_vmac_text(Bip6Vmac::new([0x00, 0x04, 0xD2])),
        "bip6:0x0004D2"
    );
}

#[test]
fn vmac_text_refuses_other_text() {
    for text in [
        "bip6:4194304",
        "bip6:4294967295",
        "bip6:99999999999999999999",
    ] {
        match parse_vmac_text(text) {
            Err(Error::OutOfRange(message)) => {
                assert!(message.contains("0..=4194303"), "{text}: {message}")
            }
            other => panic!("{text}: {other:?}"),
        }
    }
    for text in [
        "bip6:",
        "bip6:-1",
        "bip6:+1",
        "bip6:12a",
        "bip6: 1234",
        "bip6:0x",
        "bip6:0x0004D",
        "bip6:0x0004D2F",
        "bip6:0xGG0000",
        "bip6:0x+04D2",
        "BIP6:1234",
        "00:04:d2",
        "1234",
    ] {
        match parse_vmac_text(text) {
            Err(Error::Encoding(message)) => {
                assert!(
                    message.contains("bip6:0x<6 hex digits>"),
                    "{text}: {message}"
                )
            }
            other => panic!("{text}: {other:?}"),
        }
    }
}

#[test]
fn generate_random_vmac_is_nondeterministic() {
    let vmacs: Vec<Bip6Vmac> = (0..10).map(|_| generate_random_vmac().unwrap()).collect();
    let all_same = vmacs.windows(2).all(|window| window[0] == window[1]);
    assert!(!all_same, "10 random VMACs should not all be identical");
}

#[tokio::test]
async fn out_of_range_device_instance_fails_before_bip6_startup() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(0x40_0000));
    assert!(transport.start().await.is_err());
}

#[tokio::test]
async fn random_vmac_foreign_device_fails_before_bip6_startup() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port: 47_808,
        ttl: 60,
    });

    let error = transport.start().await.unwrap_err();
    assert!(
        matches!(error, Error::Transport(error) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn peer_probe_collision_fails_startup_without_publishing_socket() {
    let device_instance = 0x12_3456;
    let vmac = vmac_from_device_instance(device_instance).unwrap();
    // The peer needs the port before the start binds it, so it comes from a
    // probe. Another socket can take the port in between (#1032): that start
    // fails at the bind instead of at the collision, and the test goes again
    // with a fresh port.
    let mut attempt = 1;
    let (transport, error) = loop {
        let port = std::net::UdpSocket::bind("[::1]:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
        let peer = UdpSocket::bind("[::1]:0").await.unwrap();
        let peer_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let probe = encoded(&Bvll6::AddressResolutionAck {
                source: vmac,
                destination: vmac,
            })
            .unwrap();
            for _ in 0..20 {
                let _ = peer.send_to(&probe, destination).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, port, Some(device_instance));
        let error = transport.start().await.unwrap_err();
        peer_task.abort();
        let _ = peer_task.await;
        match error {
            Error::Transport(ref err) if lost_port(attempt, err) => attempt += 1,
            error => break (transport, error),
        }
    };

    assert!(
        matches!(&error, Error::Transport(error) if error.kind() == std::io::ErrorKind::AddrInUse
            && error.to_string() == "configured BACnet Device instance VMAC is already in use"),
        "{error:?}"
    );
    assert!(transport.socket.is_none());
    let send_error = transport.send_broadcast(&[0x01, 0x00]).await.unwrap_err();
    assert!(
        matches!(send_error, Error::Transport(error) if error.kind() == std::io::ErrorKind::NotConnected)
    );
}

#[test]
fn max_vmac_retries_constant() {
    const { assert!(MAX_VMAC_RETRIES >= 1, "must allow at least one retry") };
}
