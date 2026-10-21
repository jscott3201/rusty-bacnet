//! The BACnet/SC frame tap over a real hub and TLS (#957): a client's
//! handshake and a ReadProperty to another node on the hub reach the tap
//! as BVLC-SC messages in plaintext, though the socket under them is TLS.
#![cfg(feature = "sc-tls")]

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::{ScActiveHub, ScTransport};
use bacnet_transport::sc_frame::{decode_sc_message, ScFunction};
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_transport::tap::{TapDirection, TapPeer};
use bacnet_types::data_link::DataLink;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

const HUB_VMAC: [u8; 6] = [0x10; 6];
const CLIENT_VMAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];
const RESPONDER_VMAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0x02];
/// Real TLS on a shared runner: generous, so load cannot fail a run that
/// takes well under a second (#1811).
const WAIT: Duration = Duration::from_secs(30);

/// A CA, and a leaf under it for each name.
fn pki(
    names: &[&str],
) -> (
    CertificateDer<'static>,
    Vec<(CertificateDer<'static>, Vec<u8>)>,
) {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let key = rcgen::KeyPair::generate().unwrap();
    let ca = params.self_signed(&key).unwrap().der().clone();
    let issuer = rcgen::Issuer::from_params(&params, &key);
    let leaves = names
        .iter()
        .map(|name| {
            let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
            params.distinguished_name = rcgen::DistinguishedName::new();
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, *name);
            let key = rcgen::KeyPair::generate().unwrap();
            (
                params.signed_by(&key, &issuer).unwrap().der().clone(),
                key.serialize_der(),
            )
        })
        .collect();
    (ca, leaves)
}

async fn node(url: &str, tls: ScNodeTlsConfig, vmac: [u8; 6]) -> ScTransport<TlsWebSocket> {
    let socket = TlsWebSocket::connect(url, tls).await.unwrap();
    ScTransport::new(socket, vmac).with_device_uuid([vmac[5]; 16])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_property_over_tls_is_tapped_in_plaintext() {
    let (ca, leaves) = pki(&["hub", "client", "responder"]);
    let tls = |index: usize| {
        let (der, key) = &leaves[index];
        (
            vec![ca.clone()],
            vec![der.clone()],
            PrivatePkcs8KeyDer::from(key.clone()).into(),
        )
    };
    let (roots, chain, key) = tls(0);
    let hub_tls = ScHubTlsConfig::from_der(roots, chain, key).unwrap();
    let mut hub = ScHub::start("127.0.0.1:0", hub_tls, HUB_VMAC, [9; 16])
        .await
        .unwrap();
    let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
    let node_tls = |index| {
        let (roots, chain, key) = tls(index);
        ScNodeTlsConfig::from_der(roots, chain, key).unwrap()
    };

    let mut responder = node(&url, node_tls(2), RESPONDER_VMAC).await;
    let mut requests = responder.start().await.unwrap();
    let transport = node(&url, node_tls(1), CLIENT_VMAC).await;
    // Subscribed before the client starts the transport, to see the handshake.
    let mut frames = transport.frame_tap().subscribe();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();

    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let answer = tokio::spawn(async move {
        let request = tokio::time::timeout(WAIT, requests.recv())
            .await
            .unwrap()
            .unwrap();
        let Apdu::ConfirmedRequest(rp) =
            decode_apdu(decode_npdu(request.npdu).unwrap().payload).unwrap()
        else {
            panic!("a confirmed request");
        };
        // ComplexAck: Device 1234's Object_Name is "A".
        let reply = [
            0x01,
            0x00,
            0x30,
            rp.invoke_id,
            0x0C,
            0x0C,
            0x02,
            0x00,
            0x04,
            0xD2,
            0x19,
            0x4D,
            0x3E,
            0x72,
            0x00,
            0x41,
            0x3F,
        ];
        responder
            .send_unicast(&reply, &request.source_mac)
            .await
            .unwrap();
        responder
    });
    let ack = tokio::time::timeout(
        WAIT,
        client.read_property(
            &RESPONDER_VMAC,
            device,
            PropertyIdentifier::OBJECT_NAME,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(ack.object_identifier, device);
    let mut responder = answer.await.unwrap();

    let mut seen = Vec::new();
    while let Some(frame) = frames.try_recv() {
        assert_eq!(frame.link, DataLink::Sc);
        assert_eq!(frame.peer, TapPeer::ScHub(ScActiveHub::Primary));
        let message = decode_sc_message(&frame.bytes).expect("plaintext BVLC-SC");
        let apdu = (message.function == ScFunction::EncapsulatedNpdu).then(|| {
            match decode_apdu(decode_npdu(message.payload.clone()).unwrap().payload).unwrap() {
                Apdu::ConfirmedRequest(request) => request.service_choice,
                Apdu::ComplexAck(ack) => ack.service_choice,
                other => panic!("unexpected {other:?}"),
            }
        });
        seen.push((frame.direction, message.function, apdu));
    }
    let read_property = Some(ConfirmedServiceChoice::READ_PROPERTY);
    assert_eq!(
        seen,
        [
            (TapDirection::Outbound, ScFunction::ConnectRequest, None),
            (TapDirection::Inbound, ScFunction::ConnectAccept, None),
            (
                TapDirection::Outbound,
                ScFunction::EncapsulatedNpdu,
                read_property
            ),
            (
                TapDirection::Inbound,
                ScFunction::EncapsulatedNpdu,
                read_property
            ),
        ]
    );

    client.stop().await.unwrap();
    responder.stop().await.unwrap();
    hub.stop().await;
}
