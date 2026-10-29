//! The objects that save through `bacnet_objects::durable` keep `UnwindSafe`
//! and `RefUnwindSafe` (#1428).
//!
//! The check happens at compile time: a field that takes either trait away
//! from one of these types stops this test from building.
//!
//! Left out on purpose (#1452): the Notification Forwarder and the Audit Log
//! save through the same writer, but each holds a clock the application
//! supplies, and the log also keeps settled results whose error can wrap an
//! `std::io::Error`. Neither type has the traits; their docs say why.

// Only std unwinds and checks these traits; without it the objects lock with
// a spin lock, which has neither.
#![cfg(feature = "std")]

use std::panic::{RefUnwindSafe, UnwindSafe};

use bacnet_objects::access_control::AccessRightsObject;
use bacnet_objects::notification_class::NotificationClass;

fn assert_unwind_safe<T: UnwindSafe + RefUnwindSafe>() {}

#[test]
fn durable_objects_are_unwind_safe() {
    assert_unwind_safe::<NotificationClass>();
    assert_unwind_safe::<AccessRightsObject>();
}
