//! `DeviceIdentity`'s default Object_Name, written without `core::fmt`
//! (#2187).
use super::*;

#[test]
fn the_default_name_is_device_and_the_instance() {
    for (instance, name) in [
        (0, "device-0"),
        (9, "device-9"),
        (10, "device-10"),
        (1234, "device-1234"),
        (2_097_151, "device-2097151"),
        // The highest instance an object may have.
        (4_194_302, "device-4194302"),
    ] {
        let identity = DeviceIdentity::new(instance, 0).unwrap();
        assert_eq!(identity.name(), name, "instance {instance}");
    }
    // 4194303 is the wildcard, which no Device may have.
    assert!(DeviceIdentity::new(4_194_303, 0).is_err());
}

/// Every digit count the type can hold, against `format!`.
#[test]
fn the_digits_match_format_up_to_u32_max() {
    for instance in [99, 100, 999_999, 1_000_000, u32::MAX] {
        assert_eq!(default_name(instance), format!("device-{instance}"));
    }
}
