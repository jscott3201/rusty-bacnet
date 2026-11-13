//! Event notification request and NotificationParameters codec tests.

use super::*;
use bacnet_types::constructed::{
    BACnetDeviceObjectReference, BACnetPropertyValue, ChangeOfValueChoice,
    EventNotificationRequest, NotificationParameters,
};
use bacnet_types::enums::{
    AccessEvent, EventState, EventType, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    NotifyType, Reliability, TimerState, TimerTransition,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, StatusFlags, Time};

mod cut_short;
mod event_notification_decode;
mod notification_forms;
mod notification_parameters;
mod notification_parameters_boundaries;
mod notification_parameters_life_safety;
mod notification_parameters_reachable_wire;
mod notification_parameters_structured;
mod property_states;
mod request_round_trip;
