//! IUT capability declaration — what the device under test supports.

use std::collections::{HashMap, HashSet};

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

/// Describes what an IUT supports — used for test selection.
///
/// `services_supported` uses raw `u8` values matching the bit positions in the
/// BACnet Protocol_Services_Supported bitstring (Clause 12.11.36). This avoids
/// needing separate ConfirmedServiceChoice/UnconfirmedServiceChoice types.
#[derive(Debug, Clone, Default)]
pub struct IutCapabilities {
    pub device_instance: u32,
    pub vendor_id: u16,
    pub vendor_name: String,
    pub model_name: String,
    pub firmware_revision: String,
    pub protocol_revision: u16,
    pub protocol_version: u16,
    /// 0=BOTH, 1=TRANSMIT, 2=RECEIVE, 3=NONE
    pub segmentation_supported: u8,
    pub max_apdu_length: u16,
    pub max_segments: u8,
    /// Raw Protocol_Services_Supported bit positions (0-63).
    pub services_supported: HashSet<u8>,
    pub object_types: HashSet<ObjectType>,
    pub object_list: Vec<ObjectIdentifier>,
    pub object_details: HashMap<ObjectIdentifier, ObjectDetail>,
    pub writable_properties: HashSet<(ObjectIdentifier, PropertyIdentifier)>,
}

/// Detail about a specific object instance in the IUT.
#[derive(Debug, Clone)]
pub struct ObjectDetail {
    pub object_type: ObjectType,
    pub property_list: Vec<PropertyIdentifier>,
    pub supports_cov: bool,
    pub supports_intrinsic_reporting: bool,
    pub commandable: bool,
    pub out_of_service_writable: bool,
}
