//! Executable property metadata, and the property sets callers derive from
//! it: Property_List, the required properties and property presence.

use alloc::vec::Vec;

pub use bacnet_types::datatype::{
    ArraySize, Datatype, ElementCodec, ValueShape, ValueType, standard_value_type,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::traits::BACnetObject;

/// A property's base conformance code in its Clause 12 object table.
///
/// Conditions attached to a base code are represented separately by
/// [`PropertyPresenceCondition`]. This keeps the table code available to RPM
/// and PICS while preserving why an implemented conditional row is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyConformance {
    /// Required and readable (`R`).
    RequiredRead,
    /// Required and writable (`W`).
    RequiredWrite,
    /// Optional (`O`), including an implemented conditionally present row.
    Optional,
}

impl PropertyConformance {
    /// Whether the table code classifies this property as required.
    pub const fn is_required(self) -> bool {
        matches!(self, Self::RequiredRead | Self::RequiredWrite)
    }
}

/// Why a conditionally present property row is implemented.
///
/// Metadata describes effective rows that are already present. These values
/// retain the conformance reason; they are not predicates evaluated at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertyPresenceCondition {
    /// The row is present because the object is commandable.
    Commandable,
    /// The object reports intrinsically, and its table's footnotes make this
    /// row required of such an object: the event configuration, Event_Enable,
    /// Acked_Transitions, Notify_Type, Event_Time_Stamps and
    /// Event_Detection_Enable, for example.
    IntrinsicReportingRequired,
    /// The object reports intrinsically, and its table's footnotes only let
    /// this row be present for such an object without requiring it:
    /// Event_Message_Texts, Event_Message_Texts_Config, the
    /// Event_Algorithm_Inhibit pair and Time_Delay_Normal.
    IntrinsicReportingOptional,
    /// Audit Reporting is active, making its recipient required and writable.
    AuditReporting,
    /// An optional object-owned Audit setting is provisioned.
    ObjectAuditReporting,
    /// An optional Audit priority filter on a commandable reporting object.
    CommandableAuditReporting,
    /// The Value_Source mechanism is implemented, making its source required:
    /// on a commandable object, or on one whose noncommandable Present_Value
    /// tracks its last writer (#1552).
    ValueSourceTracking,
    /// Source tracking on a commandable object requires its source array and time.
    CommandableValueSourceTracking,
    /// The paired Active_Text and Inactive_Text option is implemented.
    PairedText,
    /// A Lighting Output has a trim, which makes its Trim_Fade_Time required
    /// (Addendum 135-2020ca part 5, #1528).
    LightingTrims,
    /// A lighting output takes its colour from a colour object, which makes
    /// Color_Reference required, and Color_Override and
    /// Override_Color_Reference too where it supports colour override
    /// (Addendum 135-2020ca part 4, #1527).
    LightingColor,
    /// A Trend Log samples a BACnet property, which its table's footnotes
    /// make the condition for requiring the window, Log_Interval and
    /// Log_DeviceObjectProperty (Table 12-29 footnotes 1 and 8, #2174).
    MonitoredBacnetProperty,
    /// The log supports clock-aligned logging, which requires Align_Intervals
    /// and Interval_Offset and is the only case that admits them (Table 12-29
    /// footnote 5, Table 12-35 footnote 3, #2174).
    ClockAlignedLogging,
    /// The Network Port is at the BACnet application protocol level, which
    /// requires Network_Number, Network_Number_Quality and APDU_Length
    /// (Table 12-71 footnote 25, #2174).
    ApplicationProtocolLevel,
    /// The Network Port isn't a point-to-point link, which requires
    /// MAC_Address (Table 12-71 footnote 3, #2174).
    NotPointToPoint,
    /// A B/IP (IPv4) Network Port, which needs its mode, UDP
    /// port, address, subnet mask, gateway and DNS servers (Table 12-71
    /// footnotes 5 and 6, #2191).
    BacnetIpPort,
    /// An IPv4 application-level port that supports B/IP multicast, which
    /// requires BACnet_IP_Multicast_Address (Table 12-71 footnote 7, #2191).
    BacnetIpMulticast,
    /// An IPv4 application-level port whose device can work through a NAT
    /// router, which requires BACnet_IP_NAT_Traversal (Table 12-71
    /// footnote 9, #2191).
    NatTraversalCapable,
    /// An IPv4 application-level port configured to work through a NAT
    /// router, which requires BACnet_IP_Global_Address (Table 12-71
    /// footnote 10, #2191).
    NatTraversalConfigured,
    /// An IPv4 or IPv6 port that can act as a BBMD, which requires its
    /// three BBMD rows (Table 12-71 footnotes 11 and 12, #2191).
    BbmdCapable,
    /// An IPv4 or IPv6 port in FOREIGN mode, which requires FD_BBMD_Address
    /// and FD_Subscription_Lifetime (Table 12-71 footnote 13, #2191).
    ForeignDeviceMode,
    /// An IPv6 application-level port, which requires its mode, UDP port
    /// and multicast address (Table 12-71 footnote 14, #2191).
    BacnetIpv6Application,
    /// An IPv6 port, which requires its address, prefix length, gateway and
    /// DNS servers (Table 12-71 footnote 15, #2191).
    Ipv6Port,
    /// The object reports COV, which its table's footnote makes the condition
    /// for COV_Increment (and for a Pulse Converter's COV_Period too): the
    /// analog types, Loop, Pulse Converter, the numeric value types, Lighting
    /// Output and Staging (#2193). A row carries it only on an object whose
    /// [`supports_cov`](crate::traits::BACnetObject::supports_cov) is true.
    CovReporting,
    /// A Timer's Present_Value is writable while it is in service, which
    /// requires Min_Pres_Value and Max_Pres_Value (Table 12-75 footnote 4,
    /// #2192).
    PresentValueWritableInService,
}

/// The write capability implemented by an object's property-write routes.
///
/// A conditional capability remains writable for PICS purposes even when the
/// current object state causes a particular request to be denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertyWriteCapability {
    /// No network property-write route is implemented.
    ReadOnly,
    /// The property-write route is available whatever Out_Of_Service and
    /// command ownership say. The object's own checks may still refuse a
    /// value, or a write in some state: an Audit Log refuses a Buffer_Size
    /// write while Log_Enable is TRUE, for example.
    Always,
    /// The property-write route is available only while Out_Of_Service is true.
    WhenOutOfService,
    /// Correction is restricted to the writer the value came from: the
    /// original command owner at that priority, or for a Present_Value with
    /// no priority array, its last writer (#1552).
    WhenCommandOwner,
    /// The property has no write route of its own, but a write of the named
    /// property changes it too: State_Text written whole, or its size at
    /// index 0, sets a multi-state object's Number_Of_States (#1443). A
    /// WriteProperty naming this property itself is refused, so it doesn't
    /// count as writable.
    Through(PropertyIdentifier),
}

impl PropertyWriteCapability {
    /// Whether a network property-write route naming this property is
    /// implemented. [`Self::Through`] has none.
    pub const fn is_writable(self) -> bool {
        !matches!(self, Self::ReadOnly | Self::Through(_))
    }

    /// The property whose writes change this one, for [`Self::Through`].
    pub const fn written_through(self) -> Option<PropertyIdentifier> {
        match self {
            Self::Through(property) => Some(property),
            _ => None,
        }
    }
}

/// Canonical metadata for one effective property row on a BACnet object.
///
/// An object's canonical set includes `PROPERTY_LIST` itself, and the
/// identity properties. [`property_list`] derives the Property_List the
/// object serves, [`required_properties`] its required properties and
/// [`has_property`] whether it has a property at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PropertyMetadata {
    /// The standard property identifier.
    pub property_identifier: PropertyIdentifier,
    /// The base `R`, `W`, or `O` table classification.
    pub conformance: PropertyConformance,
    /// The reason a conditionally present row is implemented.
    pub presence_condition: Option<PropertyPresenceCondition>,
    /// The write route implemented for this property.
    pub write_capability: PropertyWriteCapability,
    /// The value's datatype and shape, when this row states its own. `None`,
    /// what [`Self::new`] gives, takes the standard one for the object's
    /// type ([`standard_value_type`]); every built-in row does. A custom
    /// object states one for a property the standard has none for, such as
    /// a proprietary one, with [`Self::with_value_type`].
    pub value_type: Option<ValueType>,
}

impl PropertyMetadata {
    /// Whether this effective row is required, including the rows intrinsic
    /// reporting, enabled Audit Reporting and command-source mechanisms
    /// require while retaining optional base table codes.
    pub const fn is_required(self) -> bool {
        self.conformance.is_required()
            || matches!(
                self.presence_condition,
                Some(
                    PropertyPresenceCondition::IntrinsicReportingRequired
                        | PropertyPresenceCondition::AuditReporting
                        | PropertyPresenceCondition::ValueSourceTracking
                        | PropertyPresenceCondition::CommandableValueSourceTracking
                        | PropertyPresenceCondition::LightingTrims
                        | PropertyPresenceCondition::LightingColor
                        | PropertyPresenceCondition::MonitoredBacnetProperty
                        | PropertyPresenceCondition::ClockAlignedLogging
                        | PropertyPresenceCondition::ApplicationProtocolLevel
                        | PropertyPresenceCondition::NotPointToPoint
                        | PropertyPresenceCondition::BacnetIpPort
                        | PropertyPresenceCondition::BacnetIpMulticast
                        | PropertyPresenceCondition::NatTraversalCapable
                        | PropertyPresenceCondition::NatTraversalConfigured
                        | PropertyPresenceCondition::BbmdCapable
                        | PropertyPresenceCondition::ForeignDeviceMode
                        | PropertyPresenceCondition::BacnetIpv6Application
                        | PropertyPresenceCondition::Ipv6Port
                        | PropertyPresenceCondition::CovReporting
                        | PropertyPresenceCondition::PresentValueWritableInService
                )
            )
    }

    /// Construct one canonical metadata row.
    ///
    /// This constructor is the stable construction path as the non-exhaustive
    /// metadata model gains conditions needed by later object migrations.
    pub const fn new(
        property_identifier: PropertyIdentifier,
        conformance: PropertyConformance,
        presence_condition: Option<PropertyPresenceCondition>,
        write_capability: PropertyWriteCapability,
    ) -> Self {
        Self {
            property_identifier,
            conformance,
            presence_condition,
            write_capability,
            value_type: None,
        }
    }

    /// The same row, stating its own datatype and shape.
    ///
    /// ```
    /// use bacnet_objects::property_metadata::{
    ///     Datatype, PropertyConformance, PropertyMetadata, PropertyWriteCapability, ValueType,
    /// };
    /// use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    ///
    /// let proprietary = PropertyIdentifier::from_raw(512);
    /// let row = PropertyMetadata::new(
    ///     proprietary,
    ///     PropertyConformance::Optional,
    ///     None,
    ///     PropertyWriteCapability::ReadOnly,
    /// )
    /// .with_value_type(ValueType::array(Datatype::Real));
    /// let value_type = row.effective_value_type(ObjectType::from_raw(130)).unwrap();
    /// assert!(value_type.is_array());
    /// ```
    pub const fn with_value_type(self, value_type: ValueType) -> Self {
        Self {
            value_type: Some(value_type),
            ..self
        }
    }

    /// This row's datatype and shape on an object of `object_type`: its own
    /// [`Self::value_type`], or else the standard one.
    pub const fn effective_value_type(self, object_type: ObjectType) -> Option<ValueType> {
        match self.value_type {
            Some(value_type) => Some(value_type),
            None => standard_value_type(object_type, self.property_identifier),
        }
    }
}

/// The datatype and shape of `property` on an object of `object_type` with
/// these rows: its row's [`PropertyMetadata::effective_value_type`], or the
/// standard one when no row names it.
///
/// A property without a row keeps its standard shape, so an indexed read of
/// a standard array the object lacks still fails as an unknown property
/// rather than as a property that isn't an array.
pub fn value_type_of(
    object_type: ObjectType,
    rows: &[PropertyMetadata],
    property: PropertyIdentifier,
) -> Option<ValueType> {
    match rows.iter().find(|row| row.property_identifier == property) {
        Some(row) => row.effective_value_type(object_type),
        None => standard_value_type(object_type, property),
    }
}

/// The COV_Increment row of an object that reports COV, writable.
///
/// Each table that has the row ties it to COV reporting in a footnote:
/// Table 12-2 footnote 2 (Analog Input), 12-3 footnote 1 (Analog Output),
/// 12-4 footnote 2 (Analog Value), 12-20 footnote 4 (Loop), 12-27 footnote 2
/// (Pulse Converter), 12-46, 12-50 and 12-51 footnote 3 (Large Analog,
/// Integer and Positive Integer Value), 12-64 footnote 3 (Lighting Output)
/// and 12-80 footnote 3 (Staging), #2193.
pub(crate) const COV_INCREMENT_ROW: PropertyMetadata = PropertyMetadata::new(
    PropertyIdentifier::COV_INCREMENT,
    PropertyConformance::Optional,
    Some(PropertyPresenceCondition::CovReporting),
    PropertyWriteCapability::Always,
);

/// Whether `property` belongs in an object's Property_List.
///
/// Clause 12.1.1.4.1 leaves four properties out of the array:
/// Object_Identifier, Object_Name, Object_Type and Property_List itself.
/// Every other row is listed.
pub const fn in_property_list(property: PropertyIdentifier) -> bool {
    !matches!(
        property,
        PropertyIdentifier::OBJECT_IDENTIFIER
            | PropertyIdentifier::OBJECT_NAME
            | PropertyIdentifier::OBJECT_TYPE
            | PropertyIdentifier::PROPERTY_LIST
    )
}

/// The Property_List an object with these rows serves, in row order.
///
/// It is the rows' identifiers without the four that [`in_property_list`]
/// leaves out. The iterator borrows the rows and allocates nothing: count
/// it, index it with `nth`, or collect it where a list is needed.
///
/// ```
/// use bacnet_objects::analog::AnalogInputObject;
/// use bacnet_objects::property_metadata::property_list;
/// use bacnet_objects::traits::BACnetObject;
/// use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier as P};
///
/// let input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
/// let rows = input.property_metadata();
/// assert!(property_list(&rows).any(|p| p == P::PRESENT_VALUE));
/// assert!(property_list(&rows).all(|p| p != P::OBJECT_NAME));
/// ```
pub fn property_list(
    rows: &[PropertyMetadata],
) -> impl DoubleEndedIterator<Item = PropertyIdentifier> + Clone + '_ {
    rows.iter()
        .map(|row| row.property_identifier)
        .filter(|&property| in_property_list(property))
}

/// The properties these rows make required, in row order.
///
/// A row is required by its `R` or `W` code, or by the condition that made
/// it present ([`PropertyMetadata::is_required`]). Property_List is a
/// required row of every object type, so it is included; a service that
/// leaves it out, such as ReadPropertyMultiple's REQUIRED, filters it.
pub fn required_properties(
    rows: &[PropertyMetadata],
) -> impl DoubleEndedIterator<Item = PropertyIdentifier> + Clone + '_ {
    rows.iter()
        .filter(|row| row.is_required())
        .map(|row| row.property_identifier)
}

/// Whether these rows include `property`: the object has the property.
///
/// One scan of the rows, no allocation. The four properties Property_List
/// leaves out are rows like any other, so this answers `true` for them.
pub fn has_property(rows: &[PropertyMetadata], property: PropertyIdentifier) -> bool {
    rows.iter().any(|row| row.property_identifier == property)
}

/// Read an object's Property_List from its metadata rows.
///
/// The list is [`property_list`] over `rows`, one pass that leaves out the
/// four properties Clause 12.1.1.4.1 excludes. `array_index` `None` reads
/// the whole array (the one allocation, the value returned), `Some(0)` its
/// length and `Some(n)` its nth element, and an index past the end is an
/// invalid array index. A custom object's `read_property` can answer
/// Property_List with it.
pub fn read_property_list(
    rows: &[PropertyMetadata],
    array_index: Option<u32>,
) -> Result<PropertyValue, Error> {
    let mut listed = property_list(rows);
    match array_index {
        None => {
            // Sized for every row, so the list is one allocation.
            let mut elements = Vec::with_capacity(rows.len());
            elements.extend(listed.map(|p| PropertyValue::Enumerated(p.to_raw())));
            Ok(PropertyValue::List(elements))
        }
        Some(0) => Ok(PropertyValue::Unsigned(listed.count() as u64)),
        Some(index) => listed
            .nth(index as usize - 1)
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .ok_or_else(crate::common::invalid_array_index_error),
    }
}

/// The datatype and shape of `property` on `object`: [`value_type_of`] over
/// its type and metadata rows. `None` for a property with neither a row
/// value type nor a standard one.
pub fn object_value_type<O: BACnetObject + ?Sized>(
    object: &O,
    property: PropertyIdentifier,
) -> Option<ValueType> {
    value_type_of(
        object.object_identifier().object_type(),
        &object.property_metadata(),
        property,
    )
}

/// Project implemented write capability for one property identifier.
pub fn is_writable_in_metadata(
    metadata: &[PropertyMetadata],
    property_identifier: PropertyIdentifier,
) -> bool {
    metadata.iter().any(|row| {
        row.property_identifier == property_identifier && row.write_capability.is_writable()
    })
}
