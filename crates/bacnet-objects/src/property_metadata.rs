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

/// Derive all effectively required identifiers from canonical metadata.
///
/// Unlike the legacy property-list projection, this includes `PROPERTY_LIST`.
/// Consumers such as RPM apply their own service-specific exclusion.
pub fn required_properties_from_metadata(
    metadata: &[PropertyMetadata],
) -> Cow<'static, [PropertyIdentifier]> {
    Cow::Owned(
        metadata
            .iter()
            .filter_map(|row| row.is_required().then_some(row.property_identifier))
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
