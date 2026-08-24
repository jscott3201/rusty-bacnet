//! Executable property metadata and compatibility projections.

use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier;

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
    /// The row is present because the object implements intrinsic reporting.
    IntrinsicReporting,
    /// The paired Active_Text and Inactive_Text option is implemented.
    PairedText,
}

/// The write capability implemented by an object's `write_property` route.
///
/// A conditional capability remains writable for PICS purposes even when the
/// current object state causes a particular request to be denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertyWriteCapability {
    /// No network property-write route is implemented.
    ReadOnly,
    /// The property-write route is available without an object-state gate.
    Always,
    /// The property-write route is available only while Out_Of_Service is true.
    WhenOutOfService,
}

impl PropertyWriteCapability {
    /// Whether any network property-write route is implemented.
    pub const fn is_writable(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

/// Canonical metadata for one effective property row on a BACnet object.
///
/// A migrated object's canonical set includes `PROPERTY_LIST` itself. The
/// legacy [`BACnetObject::property_list`](crate::traits::BACnetObject::property_list)
/// projection omits that identifier, and the encoded BACnet Property_List
/// value additionally omits Object_Identifier, Object_Name, and Object_Type.
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
}

impl PropertyMetadata {
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
        }
    }
}

/// Derive the legacy object property-list projection from canonical metadata.
///
/// The projection preserves metadata order and omits only `PROPERTY_LIST`.
pub fn property_list_from_metadata(
    metadata: &[PropertyMetadata],
) -> Cow<'static, [PropertyIdentifier]> {
    Cow::Owned(
        metadata
            .iter()
            .filter_map(|row| {
                (row.property_identifier != PropertyIdentifier::PROPERTY_LIST)
                    .then_some(row.property_identifier)
            })
            .collect(),
    )
}

/// Derive all table-required identifiers from canonical metadata.
///
/// Unlike the legacy property-list projection, this includes `PROPERTY_LIST`.
/// Consumers such as RPM apply their own service-specific exclusion.
pub fn required_properties_from_metadata(
    metadata: &[PropertyMetadata],
) -> Cow<'static, [PropertyIdentifier]> {
    Cow::Owned(
        metadata
            .iter()
            .filter_map(|row| {
                row.conformance
                    .is_required()
                    .then_some(row.property_identifier)
            })
            .collect(),
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
