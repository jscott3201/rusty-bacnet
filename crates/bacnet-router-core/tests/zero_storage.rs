//! Empty storage is all zero bits, whatever its sizes (#2679).
//!
//! That is what lets firmware keep a router's [`Storage`] in a
//! zero-initialized static (`.bss`), with no image of it in flash, and what
//! lets a host fill a box with it as zeroed memory. Until #2679 it held
//! only without a duplicate record: an empty slot of the record carried a
//! tag of 0xFF, so storage with the default record of 32 had 32 octets
//! that were not zero.
//!
//! Each case is a static, and the test reads the image the compiler made
//! for it, octet by octet: the same octets a linker looks at to place it.
//! The firmware link test checks the same thing for the one storage its
//! router image keeps, which has no record
//! (`scripts/ci/check-firmware-link.sh`, the `.data` budget).

use bacnet_router_core::Storage;
use bacnet_router_core::npdu::MAX_NPDU;
use bacnet_router_core::router::Groups;

/// The default searches, scratch, group rule and duplicate record (32).
static DEFAULT_SIZES: Storage<4, 16, 16> = Storage::new();

/// No duplicate record, as the firmware router image keeps it.
static NO_RECORD: Storage<2, 8, 8, 4, MAX_NPDU, Groups, 0> = Storage::new();

/// A duplicate record of another size, with other sizes around it.
static RECORD_OF_7: Storage<3, 5, 2, 1, 1600, Groups, 7> = Storage::new();

/// A record as large as the hosted router's, in a larger scratch.
static RECORD_OF_512: Storage<8, 32, 32, 16, 4096, Groups, 512> = Storage::new();

/// The octets of a static that are not zero, as `(offset, octet)`.
#[allow(unsafe_code)]
fn nonzero<T: Sync>(storage: &'static T) -> Vec<(usize, u8)> {
    // Through `black_box`, so the octets are read from the static's image
    // and not from what the compiler knows of its value.
    let start = std::hint::black_box(std::ptr::from_ref(storage)).cast::<u8>();
    // SAFETY: `storage` is a static, so `start` points at `size_of::<T>()`
    // octets that live for the whole program and that nothing writes: the
    // statics here have no interior mutability. Its padding is read with
    // the rest, as the compiler emitted it into the image, because padding
    // that is not zero keeps a static out of `.bss` as surely as a field
    // does.
    let image = unsafe { std::slice::from_raw_parts(start, size_of::<T>()) };
    image
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, octet)| octet != 0)
        .collect()
}

#[test]
fn empty_storage_of_the_default_sizes_is_all_zero_bits() {
    assert_eq!(nonzero(&DEFAULT_SIZES), []);
}

#[test]
fn empty_storage_without_a_duplicate_record_is_all_zero_bits() {
    assert_eq!(nonzero(&NO_RECORD), []);
}

#[test]
fn empty_storage_with_a_duplicate_record_of_any_size_is_all_zero_bits() {
    assert_eq!(nonzero(&RECORD_OF_7), []);
    assert_eq!(nonzero(&RECORD_OF_512), []);
}
