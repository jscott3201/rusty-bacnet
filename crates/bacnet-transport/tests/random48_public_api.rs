//! Public Random-48 VMAC API tests.

use bacnet_transport::sc::generate_random48_vmac;
use bacnet_transport::sc_frame::{is_valid_random48_vmac, BROADCAST_VMAC, UNKNOWN_VMAC};

#[test]
fn public_random48_generator_is_callable_and_shape_valid() {
    let vmac = generate_random48_vmac().unwrap();

    assert!(is_valid_random48_vmac(&vmac));
    assert_eq!(vmac[0] & 0x0F, 0x02);
    assert_ne!(vmac, UNKNOWN_VMAC);
    assert_ne!(vmac, BROADCAST_VMAC);
}

#[test]
fn public_random48_shape_predicate_checks_marker_nibble() {
    assert!(is_valid_random48_vmac(&[
        0xF2, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE
    ]));

    for marker in 0x00..=0x0F {
        let mut vmac = [0xF0 | marker, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        assert_eq!(
            is_valid_random48_vmac(&vmac),
            marker == 0x02,
            "low nibble {marker:#04x} should be the only Random-48 marker"
        );

        vmac[0] = marker;
        assert_eq!(
            is_valid_random48_vmac(&vmac),
            marker == 0x02,
            "high nibble must not affect Random-48 marker validation"
        );
    }

    assert!(!is_valid_random48_vmac(&UNKNOWN_VMAC));
    assert!(!is_valid_random48_vmac(&BROADCAST_VMAC));
}
