mod enrollment_summary_projection;
#[cfg(feature = "std")]
mod fold_tests;
mod lookup;
#[cfg(feature = "std")]
mod persistence_tests;
mod priority;
mod properties;
mod recipient_list;
mod recipient_list_cap;
mod recipients;
mod routed_list;
#[cfg(feature = "std")]
mod staging_tests;
#[cfg(feature = "std")]
mod storage;

use super::*;
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::Time;

fn make_time(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn make_dest_device(device_instance: u32) -> BACnetDestination {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap();
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: make_time(0, 0),
        to_time: make_time(23, 59),
        recipient: BACnetRecipient::Device(dev_oid),
        process_identifier: 1,
        issue_confirmed_notifications: true,
        transitions: EventTransitionBits::all(),
    }
}
