//! The default Object_Names of the Device and of a declared Network Port,
//! written without `core::fmt` (#2187) by `common::numbered_name`.
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
        assert_eq!(
            numbered_name("device-", instance),
            format!("device-{instance}")
        );
    }
}

/// A declared Network Port's object is named `port-<instance>`, across the
/// instances a port may have (1 to 255), and the shared writer gives the
/// same form at 0 and at the highest object instance.
#[test]
fn a_declared_port_is_named_port_and_the_instance() {
    for (instance, name) in [(1, "port-1"), (128, "port-128"), (255, "port-255")] {
        let entry = NetworkPortEntry::bip(instance, 1, [192, 168, 1, 10].into(), 47808);
        let port = entry.build_object().unwrap();
        assert_eq!(port.object_name(), name, "instance {instance}");
    }
    for (instance, name) in [
        (0, "port-0"),
        (2_097_151, "port-2097151"),
        (4_194_302, "port-4194302"),
    ] {
        assert_eq!(numbered_name("port-", instance), name);
    }
}
