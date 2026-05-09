//! Service handlers for incoming BACnet requests.
//!
//! Each handler function processes a decoded service request against an
//! ObjectDatabase and returns the encoded response bytes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, EventSummary, GetEventInformationAck, GetEventInformationRequest,
};
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{
    ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_services::who_has::{IHaveRequest, WhoHasObject, WhoHasRequest};
use bacnet_services::wpm::WritePropertyMultipleRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{
    EnableDisable, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

/// Property identifier for File Data (property 65 / 0x41).
const PROP_FILE_DATA: u32 = 0x0041;
use bytes::BytesMut;

use crate::cov::{CovSubscription, CovSubscriptionTable};

mod alarm_event;
mod cov;
mod device_mgmt;
mod file;
mod list;
mod object_mgmt;
mod read_property;
mod write_group;
mod write_property;

pub use alarm_event::*;
pub use cov::*;
pub use device_mgmt::*;
pub use file::*;
pub use list::*;
pub use object_mgmt::*;
pub use read_property::*;
pub use write_group::*;
pub use write_property::*;

#[cfg(test)]
mod tests;
