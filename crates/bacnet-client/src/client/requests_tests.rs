use super::{check_transmittable_length, LengthBoundedBy};

#[test]
fn conformant_lengths_pass() {
    for advertised in [50u16, 128, 206, 480, 1024, 1476] {
        assert!(
            check_transmittable_length(LengthBoundedBy::DiscoveredPeer(advertised), 1476).is_ok(),
            "{advertised} is at or above MinimumMessageSize"
        );
    }
}

/// I-Am carries an Unsigned octet count, not the four-bit code, and Clause
/// 20.1.2.5 says the true value "may be larger than indicated in this
/// parameter" — so values outside the six encodings are legitimate.
#[test]
fn lengths_outside_the_encoded_set_are_not_rejected() {
    for advertised in [51u16, 600, 1500, u16::MAX] {
        assert!(
            check_transmittable_length(LengthBoundedBy::DiscoveredPeer(advertised), u16::MAX)
                .is_ok(),
            "{advertised} is conformant even though it is not one of the six encodings"
        );
    }
}

/// The error must blame whichever term actually bound the minimum. The peer
/// is only sometimes that term: BACnet/SC recomputes its own limit from the
/// hub's Connect-Accept, so a transport can fall below the floor while the
/// peer is entirely conformant.
#[test]
fn the_error_names_the_binding_term() {
    let peer_bound =
        check_transmittable_length(LengthBoundedBy::DiscoveredPeer(3), 1476).unwrap_err();
    assert!(
        peer_bound.to_string().contains("peer's advertised"),
        "peer is the binding term here, got: {peer_bound}"
    );

    let transport_bound =
        check_transmittable_length(LengthBoundedBy::DiscoveredPeer(1476), 48).unwrap_err();
    assert!(
        transport_bound.to_string().contains("transport's limit"),
        "transport is the binding term here and the peer is conformant, got: {transport_bound}"
    );
    assert!(
        !transport_bound.to_string().contains("peer's advertised"),
        "must not blame a conformant peer for the transport's limit"
    );
}
