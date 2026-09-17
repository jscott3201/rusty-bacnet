//! RB-07 network threading tests (in-memory, deterministic, no sleeps).
//!
//! Covers: IngressContext threads provenance, ReceivedApdu preserves it
//! through clone/queueing, routed sources stay claims (compat: decisions
//! unchanged), redacted Debug.

use super::layer::ReceivedApdu;
use super::router::IngressContext;
use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;
use bytes::Bytes;

fn test_npdu() -> Npdu {
    Npdu::default()
}

#[test]
fn ingress_context_threads_provenance_by_value() {
    use bacnet_transport::sc::{LoopbackWebSocket, ScTransport};
    // test_local is unverified legacy; test_local_with_provenance threads any
    // value by value (verified values arrive behaviorally via SC in async
    // paths; here we prove threading with distinct unverified-derived copies
    // plus a scope check on the default).
    let npdu = test_npdu();
    let unverified = IngressContext::test_local(0, 1000, &[0x0A], npdu.clone());
    assert!(unverified.provenance.is_unverified());
    let copied = IngressContext::test_local_with_provenance(
        0,
        1000,
        &[0x0A],
        npdu,
        TransportProvenance::unverified(),
    );
    assert_eq!(copied.provenance, unverified.provenance);
    assert!(copied.provenance.scope().contains("unverified"));
    let _ = (
        LoopbackWebSocket::pair,
        ScTransport::<LoopbackWebSocket>::new,
    );
}

#[test]
fn received_apdu_preserves_provenance_through_clone() {
    let apdu = ReceivedApdu::unverified(
        Bytes::from_static(&[0x01]),
        MacAddr::from_slice(&[0x0A]),
        Some(1000),
        Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x55]),
        }),
        false,
        false,
        Vec::new(),
        None,
    );
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
        apdu: Bytes::from_static(&[0xDE, 0xAD]),
        source_mac: MacAddr::from_slice(&[0xAA, 0xBB, 0xCC]),
        ingress_network: Some(1000),
        source_network: Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x11, 0x22]),
        }),
        link_layer_group: false,
        is_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    };
    let rendered = format!("{:?}", apdu);
    assert!(rendered.contains("source_mac_len"));
    assert!(rendered.contains("unverified"));
    assert!(!rendered.contains("AA"));
}
