//! The B/IPv6 VMAC a Device instance gives (Clause H.7.2). The transport
//! takes it as its own address, and the server and endpoint check a
//! registered port's against the Device they serve (#2057), so it lives
//! here, outside the transport's `ipv6` feature. It allocates nothing.

use core::fmt;

/// Highest Device instance, whose 22 bits make a VMAC.
pub const MAX_DEVICE_INSTANCE: u32 = 0x3F_FFFF;

/// The 3-octet VMAC Device instance `instance` gives a B/IPv6 node: the
/// instance's 22 bits, high octet first, which leaves the top two bits 00
/// (H.7.2). `None` for an instance above 4194303.
///
/// ```
/// use bacnet_types::bip6_vmac::device_instance_vmac;
///
/// assert_eq!(device_instance_vmac(1234), Some([0x00, 0x04, 0xD2]));
/// assert_eq!(device_instance_vmac(4_194_304), None);
/// ```
pub const fn device_instance_vmac(instance: u32) -> Option<[u8; 3]> {
    if instance > MAX_DEVICE_INSTANCE {
        return None;
    }
    let [_, high, middle, low] = instance.to_be_bytes();
    Some([high, middle, low])
}

/// A B/IPv6 VMAC written as `0x` and six uppercase hex digits.
///
/// ```
/// use bacnet_types::bip6_vmac::VmacHex;
///
/// assert_eq!(VmacHex([0x00, 0x04, 0xD2]).to_string(), "0x0004D2");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmacHex(pub [u8; 3]);

impl fmt::Display for VmacHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [high, middle, low] = self.0;
        write!(f, "0x{high:02X}{middle:02X}{low:02X}")
    }
}
