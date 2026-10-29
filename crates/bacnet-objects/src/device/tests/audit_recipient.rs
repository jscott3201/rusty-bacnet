use super::*;
use bacnet_types::MacAddr;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::error::RemoteError;

/// An address recipient on the local network whose MAC is `len` octets.
fn address(len: usize) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&vec![0x0A; len]),
    })
}

#[test]
fn provisioned_audit_recipient_mac_longer_than_the_bound_is_refused() {
    // #1124: the provisioned recipient obeys the recipient codec's MAC bound,
    // with the code a WriteProperty of the same value gets.
    let mut dev = make_device();
    match dev.provision_audit_recipient(address(BACnetAddress::MAX_MAC_LEN + 1)) {
        Err(Error::Remote(RemoteError::Error { class, code })) => assert_eq!(
            (class, code),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
        ),
        other => panic!("expected PROPERTY/INVALID_DATA_ENCODING, got {other:?}"),
    }
    dev.provision_audit_recipient(address(BACnetAddress::MAX_MAC_LEN))
        .unwrap();
}
