//! `ValueSourceTracking::command` against the implementation it replaced
//! (#2342), which copied the published source before and after the command
//! and compared the two with [`ValueSourceTracking::settle`]. That oracle is
//! still made of live parts (`effect`, `record` and `settle`, which Lighting
//! uses), so a seeded random walk runs both side by side and requires the
//! same slots, sources, owners and Last_Command_Time after every step.

use super::super::CommandDeviceBinding;
use super::*;
use bacnet_types::MacAddr;
use bacnet_types::constructed::{BACnetAddress, BACnetDeviceObjectReference};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

/// The command as it was: copy the published source, command, copy it again.
fn oracle_command(
    tracking: &mut ValueSourceTracking,
    slots: &mut [Option<u8>; 16],
    value: Option<u8>,
    priority: Option<u8>,
    origin: &CommandOrigin,
) -> Result<(), Error> {
    let index = ValueSourceTracking::admit(priority, origin)?;
    let before = tracking.effect(slots, 0);
    slots[index] = value;
    tracking.record(index, origin);
    let after = tracking.effect(slots, 0);
    tracking.settle(before, after);
    Ok(())
}

/// Lighting's sourced write: it records the writer only when the act took
/// effect, and an application command names no writer.
fn lighting_write(
    tracking: &mut ValueSourceTracking,
    slots: &mut [Option<u8>; 16],
    index: usize,
    value: Option<u8>,
    origin: Option<&CommandOrigin>,
    acted: bool,
) {
    let before = tracking.effect(slots, 0);
    if acted {
        slots[index] = value;
        match origin {
            Some(origin) => tracking.record(index, origin),
            None => tracking.record_unsourced(index),
        }
    }
    let after = tracking.effect(slots, 0);
    tracking.settle(before, after);
}

/// A small deterministic generator (xorshift64*), so the walk is the same
/// on every run and needs no dependency.
struct Steps(u64);
impl Steps {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> usize {
        (self.next() % n) as usize
    }
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// Writers: remote ones whose MACs share a prefix, are inline or spilled and
/// bind to no Device, one or several; local ones with and without an
/// initiating object. Two pairs publish the same source from different
/// owners, so "same source, other writer" is covered too.
fn writers() -> Vec<CommandOrigin> {
    let remote = |mac: &[u8], network_number, binding| CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number,
            mac_address: MacAddr::from_slice(mac),
        },
        binding,
    };
    vec![
        remote(&[10, 0, 0, 1, 0xBA, 0xC0], 0, CommandDeviceBinding::Unknown),
        remote(&[10, 0, 0, 2, 0xBA, 0xC0], 0, CommandDeviceBinding::Unknown),
        remote(
            &[10, 0, 0, 2, 0xBA, 0xC0],
            7,
            CommandDeviceBinding::Ambiguous,
        ),
        remote(
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            7,
            CommandDeviceBinding::Unknown,
        ),
        remote(
            &[10, 0, 0, 3, 0xBA, 0xC0],
            0,
            CommandDeviceBinding::Unique(device(5)),
        ),
        remote(
            &[10, 0, 0, 4, 0xBA, 0xC0],
            0,
            CommandDeviceBinding::Unique(device(5)),
        ),
        CommandOrigin::Local {
            owner_device: device(1),
            initiating_object: None,
        },
        CommandOrigin::Local {
            owner_device: device(2),
            initiating_object: Some(device(1)),
        },
    ]
}

#[test]
fn command_matches_the_copying_implementation_over_a_random_walk() {
    let writers = writers();
    for seed in [0x5EED_0001_u64, 0x2342, 0xDEAD_BEEF_CAFE_F00D] {
        let mut steps = Steps(seed);
        let (mut new, mut old) = (
            ValueSourceTracking::default(),
            ValueSourceTracking::default(),
        );
        let (mut new_slots, mut old_slots) = ([None; 16], [None; 16]);
        for step in 0..20_000 {
            // Few priorities and values, so slots collide, stay active, are
            // relinquished and fall back to a lower slot often.
            let priority =
                [Some(1), Some(8), Some(9), Some(16), None, Some(0), Some(17)][steps.below(7)];
            let value = [None, Some(1), Some(2)][steps.below(3)];
            let origin = &writers[steps.below(writers.len() as u64)];
            match steps.below(10) {
                0..=6 => {
                    let a = new.command(&mut new_slots, 0, value, priority, origin);
                    let b = oracle_command(&mut old, &mut old_slots, value, priority, origin);
                    assert_eq!(a.is_ok(), b.is_ok(), "seed {seed:#x} step {step}");
                }
                7 => {
                    // Lighting: sourced or application write, acted or not.
                    let index = steps.below(16);
                    let origin = (steps.below(3) != 0).then_some(origin);
                    let acted = steps.below(4) != 0;
                    lighting_write(&mut new, &mut new_slots, index, value, origin, acted);
                    lighting_write(&mut old, &mut old_slots, index, value, origin, acted);
                }
                8 => {
                    // A correction: the slot's source differs from what its
                    // owner would publish (the source `command` replaces).
                    let index = steps.below(16);
                    let claim = BACnetValueSource::Object(BACnetDeviceObjectReference {
                        device_identifier: None,
                        object_identifier: device(9),
                    });
                    new.sources[index] = claim.clone();
                    old.sources[index] = claim;
                }
                _ => {
                    // Lighting's egress: the value moves with no writer.
                    let index = steps.below(16);
                    for (tracking, slots) in
                        [(&mut new, &mut new_slots), (&mut old, &mut old_slots)]
                    {
                        let before = tracking.effect(slots, 0);
                        slots[index] = value;
                        let after = tracking.effect(slots, 0);
                        tracking.settle(before, after);
                    }
                }
            }
            assert_eq!(new_slots, old_slots, "seed {seed:#x} step {step}");
            assert_eq!(new.sequence, old.sequence, "seed {seed:#x} step {step}");
            assert_eq!(new.sources, old.sources, "seed {seed:#x} step {step}");
            assert_eq!(new.owners, old.owners, "seed {seed:#x} step {step}");
        }
        // The walk moved Last_Command_Time, so it exercised the comparison.
        assert_ne!(new.sequence, 0, "seed {seed:#x}");
    }
}
