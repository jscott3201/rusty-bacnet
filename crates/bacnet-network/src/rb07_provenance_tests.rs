//! RB-07 network threading tests (in-memory, deterministic, no sleeps).
//!
//! Covers: ReceivedApdu preserves provenance through clone/queueing,
//! routed sources stay claims (compat: decisions unchanged), redacted Debug.
//! The router threads a frame's provenance to its consumers: see
//! `router::tests::provenance_threads_by_value_to_every_consumer`.

use super::layer::ReceivedApdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;
use bytes::Bytes;

#[test]
fn received_apdu_preserves_provenance_through_clone() {
    let apdu = ReceivedApdu {
        apdu: Bytes::from_static(&[0x01]),
        source_mac: MacAddr::from_slice(&[0x0A]),
        ingress_network: Some(1000),
        source_network: Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x55]),
        }),
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        router_response: None,
        direct_response: None,
        reply_tx: None,
    };
    assert!(apdu.provenance.is_unverified());
    // Claimed SNET/SADR stays available as a claim, never a credential.
    assert_eq!(apdu.source_network.as_ref().unwrap().network, 200);
    let cloned = apdu.clone();
    assert_eq!(cloned.provenance, apdu.provenance);
    assert!(cloned.reply_tx.is_none());
}

#[test]
fn received_apdu_debug_is_redacted() {
    let apdu = ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: Bytes::from_static(&[0xDE, 0xAD]),
        source_mac: MacAddr::from_slice(&[0xAA, 0xBB, 0xCC]),
        ingress_network: Some(1000),
        source_network: Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x11, 0x22]),
        }),
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    };
    let rendered = format!("{:?}", apdu);
    assert!(rendered.contains("source_mac_len"));
    assert!(rendered.contains("unverified"));
    assert!(!rendered.contains("AA"));
}
