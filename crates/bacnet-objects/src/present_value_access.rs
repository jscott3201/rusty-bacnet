//! How a Value object's Present_Value is written.

use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier;

use crate::property_metadata::{
    PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};

/// How peers and the local application write a Value object's Present_Value.
///
/// Priority_Array, Relinquish_Default, Current_Command_Priority,
/// Value_Source_Array and Last_Command_Time are present only under
/// [`Commandable`](Self::Commandable). Value_Source is too, unless the object
/// tracks the source of its noncommandable Present_Value (`set_value_source_tracking`
/// on each Value object, #1552).
/// Under every access, Out_Of_Service TRUE keeps the local application from
/// changing Present_Value and lets peers write it for testing (Clause 12,
/// Out_Of_Service of each Value object).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PresentValueAccess {
    /// The application sets Present_Value. Peers write it only while
    /// Out_Of_Service is TRUE.
    ReadOnly,
    /// A peer write replaces Present_Value, and so does the application.
    Writable,
    /// Peer writes go through Priority_Array, and the highest occupied
    /// priority is Present_Value (Clause 19.2).
    #[default]
    Commandable,
}

impl PresentValueAccess {
    /// Whether a row with this presence condition exists under this access.
    /// `source_tracked` says whether a noncommandable object tracks its
    /// Present_Value's source, which keeps its Value_Source row.
    pub(crate) fn includes(
        self,
        condition: Option<PropertyPresenceCondition>,
        source_tracked: bool,
    ) -> bool {
        use PropertyPresenceCondition as C;
        self == Self::Commandable
            || match condition {
                Some(
                    C::Commandable
                    | C::CommandableAuditReporting
                    | C::CommandableValueSourceTracking,
                ) => false,
                Some(C::ValueSourceTracking) => source_tracked,
                _ => true,
            }
    }

    /// `rows` as this access presents them: the rows it excludes removed, and
    /// Present_Value's write capability set to match.
    pub(crate) fn project(
        self,
        rows: Cow<'_, [PropertyMetadata]>,
        source_tracked: bool,
    ) -> Cow<'_, [PropertyMetadata]> {
        let present_value = match self {
            Self::Commandable => return rows,
            Self::Writable => PropertyWriteCapability::Always,
            Self::ReadOnly => PropertyWriteCapability::WhenOutOfService,
        };

        Cow::Owned(
            rows.iter()
                .filter(|row| self.includes(row.presence_condition, source_tracked))
                .map(|row| {
                    if row.property_identifier == PropertyIdentifier::PRESENT_VALUE {
                        PropertyMetadata::new(
                            row.property_identifier,
                            row.conformance,
                            row.presence_condition,
                            present_value,
                        )
                    } else {
                        *row
                    }
                })
                .collect(),
        )
    }

    /// Whether `rows` hold `property` under a condition this access excludes.
    pub(crate) fn excludes(
        self,
        rows: &[PropertyMetadata],
        property: PropertyIdentifier,
        source_tracked: bool,
    ) -> bool {
        self != Self::Commandable
            && rows.iter().any(|row| {
                row.property_identifier == property
                    && !self.includes(row.presence_condition, source_tracked)
            })
    }
}
