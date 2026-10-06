//! Value_Source for a Present_Value with no priority array (Clause 19.5,
//! #1552).
//!
//! A noncommandable Present_Value has one source to report: whoever made
//! its last write. The writer is recorded as the source and as the owner,
//! and only that owner may then correct the published source, under the
//! rules a commandable slot's owner follows
//! ([`CommandOrigin::permits_correction_by`]). A correction changes the
//! source and keeps the owner; the next write replaces both. Such an object
//! has no Value_Source_Array, and Clause 19.5.1.4 keeps Last_Command_Time to
//! objects with a priority array, so neither is served.
//!
//! Tracking is off until the object provisions it. Off, the object serves
//! no Value_Source row and every method here leaves its behaviour alone.

use super::{tracking::decode_claim, CommandOrigin};
use crate::common;
use crate::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};
use bacnet_encoding::constructed::encode_value_source;
use bacnet_types::{
    constructed::BACnetValueSource, enums::PropertyIdentifier as P, error::Error,
    primitives::PropertyValue,
};
use bytes::BytesMut;

/// The source of a noncommandable Present_Value, once tracking is on.
#[derive(Debug, Clone, Default)]
pub(crate) struct SingleValueSource {
    /// `None` while tracking is off.
    tracked: Option<Tracked>,
}

#[derive(Debug, Clone)]
struct Tracked {
    /// The published Value_Source: the last writer, or its correction.
    source: BACnetValueSource,
    /// The last writer, who alone may correct `source`. `None` before the
    /// first sourced write, and after a write that named no writer.
    owner: Option<CommandOrigin>,
}

impl SingleValueSource {
    /// Turn tracking on or off. Turning it on starts from source NONE with
    /// no owner, and turning it on again keeps what it holds.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.tracked = None;
        } else if self.tracked.is_none() {
            self.tracked = Some(Tracked {
                source: BACnetValueSource::None,
                owner: None,
            });
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.tracked.is_some()
    }

    /// The Value_Source row, while tracking is on. It is required then
    /// (the tables' value-source footnote), and writable by its owner.
    pub(crate) fn metadata(&self) -> Option<PropertyMetadata> {
        self.tracked.as_ref().map(|_| {
            PropertyMetadata::new(
                P::VALUE_SOURCE,
                PropertyConformance::Optional,
                Some(PropertyPresenceCondition::ValueSourceTracking),
                PropertyWriteCapability::WhenCommandOwner,
            )
        })
    }

    /// Check a write's origin before the write changes anything, as a
    /// commandable command does. Nothing to check while tracking is off.
    pub(crate) fn admit(&self, origin: &CommandOrigin) -> Result<(), Error> {
        match self.tracked {
            Some(_) => origin.validate(),
            None => Ok(()),
        }
    }

    /// A write that set Present_Value: its writer becomes the source and
    /// the owner.
    pub(crate) fn record(&mut self, origin: &CommandOrigin) {
        if let Some(tracked) = &mut self.tracked {
            tracked.source = origin.published_source();
            tracked.owner = Some(origin.clone());
        }
    }

    /// Refuse a network-equivalent write that names no writer while
    /// tracking is on, as a commandable object refuses a context-free
    /// command: the source it would leave can't be known.
    pub(crate) fn unsourced(&self) -> Result<(), Error> {
        match self.tracked {
            Some(_) => Err(common::write_access_denied_error()),
            None => Ok(()),
        }
    }

    /// The application set Present_Value through a typed setter that names
    /// no writer: no source is known, and nobody may correct it.
    pub(crate) fn forget(&mut self) {
        if let Some(tracked) = &mut self.tracked {
            tracked.source = BACnetValueSource::None;
            tracked.owner = None;
        }
    }

    /// A write of Value_Source itself (Clause 19.5.1.3): the last writer
    /// correcting the source it published. Any other origin, and any origin
    /// before the first sourced write, is WRITE_ACCESS_DENIED, checked
    /// before the value; a value that isn't one complete BACnetValueSource
    /// is INVALID_DATA_TYPE. With tracking off the property is absent.
    pub(crate) fn correct(
        &mut self,
        index: Option<u32>,
        value: PropertyValue,
        origin: &CommandOrigin,
    ) -> Result<(), Error> {
        let Some(tracked) = &mut self.tracked else {
            return Err(common::unknown_property_error());
        };
        if index.is_some() {
            return Err(common::property_is_not_an_array_error());
        }
        origin.validate()?;
        if !tracked
            .owner
            .as_ref()
            .is_some_and(|owner| owner.permits_correction_by(origin))
        {
            return Err(common::write_access_denied_error());
        }
        tracked.source = decode_claim(value)?;
        Ok(())
    }

    /// Read Value_Source as its encoded CHOICE; `None` for any other
    /// property.
    pub(crate) fn read(
        &self,
        property: P,
        index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        if property != P::VALUE_SOURCE {
            return None;
        }
        Some(match &self.tracked {
            None => Err(common::unknown_property_error()),
            Some(_) if index.is_some() => Err(common::property_is_not_an_array_error()),
            Some(tracked) => {
                let mut bytes = BytesMut::new();
                encode_value_source(&mut bytes, &tracked.source)
                    .map(|()| PropertyValue::ApplicationData(bytes.to_vec()))
            }
        })
    }
}

#[cfg(test)]
#[path = "single_tests.rs"]
mod tests;
