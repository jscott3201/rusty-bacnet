//! The application's route to Present_Value and Tracking_Value once a server
//! holds the Point or Zone (#1123).
//!
//! The inherent `set_present_value` and `set_tracking_value` need the
//! concrete type, which the database's `Box<dyn BACnetObject>` hides. The
//! object's `LocalInput` carries the same updates through the box as its
//! Present_Value and Tracking_Value inputs; `BACnetServer::set_present_value_local`
//! and `set_tracking_value_local` apply them and then run the server's COV and
//! event processing.
//!
//! Both inputs take an Enumerated holding a standard BACnetLifeSafetyState or
//! one from the proprietary range 256..=65535, the range a reset commit and an
//! out-of-service Tracking_Value write already enforce. A reserved or larger
//! number fails with PROPERTY / VALUE_OUT_OF_RANGE, any other datatype with
//! PROPERTY / INVALID_DATA_TYPE, and the object is left untouched.
//!
//! How the route fits the rest of the object:
//!
//! - Latching: Clauses 12.15.4 and 12.16.4 leave the derivation of
//!   Present_Value, and whether it holds a non-NORMAL state until a reset, to
//!   the implementation. The application owns that rule here. The object never
//!   computes one property from the other and each input sets only its own, so
//!   an application that latches keeps Present_Value on the alarm state while
//!   it goes on reporting the live state through Tracking_Value, and its reset
//!   executor commits the Present_Value that follows the reset.
//! - Silenced and Operation_Expected: neither input changes them. A new state
//!   doesn't unsilence the object or arm a reset by itself, just as a reset
//!   commit infers nothing it wasn't given. The application arms a reset
//!   through its `LifeSafety` capability's `set_life_safety_operation_expected`.
//! - Reset executor: the route takes values whether or not a reset is armed.
//!   The executor and the route are two writers of the same fields, ordered by
//!   the database lock, and the executor's context carries whatever values the
//!   route left.
//! - Out_Of_Service: Tables 12-18 and 12-19 give Present_Value no
//!   out-of-service footnote and Clauses 12.15.11 and 12.16.11 don't decouple
//!   it, so clients never write it and the Present_Value input serves the
//!   new value at once in either state. The Inputs refuse their application
//!   value while out of service because a client owns Present_Value then;
//!   nothing here competes for it. Tracking_Value is decoupled while out of
//!   service (`out_of_service.rs`), so the Tracking_Value input replaces
//!   the value set aside, the client's simulated value keeps being served, and
//!   the application's value takes over on the return to service.
//! - Event reporting: these objects run no intrinsic reporting and Event_State
//!   stays NORMAL, so neither update raises an event. The server's local write
//!   path still runs its post-write event evaluation after both routes, as for
//!   every other `set_present_value_local` caller, so a custom Life Safety
//!   object with intrinsic reporting is evaluated there.

use bacnet_types::enums::LifeSafetyState;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;

/// Decode a Present_Value or Tracking_Value the application or a client
/// supplies: an Enumerated in the state range, or the error to answer.
pub(super) fn life_safety_state(value: &PropertyValue) -> Result<LifeSafetyState, Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    let state = LifeSafetyState::from_raw(*raw);
    if !super::valid_life_safety_state(state) {
        return Err(common::value_out_of_range_error());
    }
    Ok(state)
}
