//! COV-multiple reference preparation shared by notification building and
//! timestamped change capture, so both apply one reporting criterion.
use std::collections::HashMap;

use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov_multiple::COVNotificationValue;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::flags::PreparedFlags;
use super::value_source::{self, PreparedValueSource};
use super::{CovObservation, CovSubscription};

/// One reference's current values and the observation they establish, read
/// once; [`MultipleReads::reports`] later qualifies it without object reads.
pub(crate) struct PreparedReference {
    /// Conveyed values without Time_Of_Change.
    pub values: Vec<COVNotificationValue>,
    /// Baseline established by conveying these values.
    pub observation: CovObservation,
    oid: ObjectIdentifier,
    criterion: Option<super::prepare::PreparedCovValue>,
}

/// Per-object reads shared by every reference in one preparation. Each object's
/// Status_Flags and specialized Value_Source report are read once, so sibling
/// selectors never observe two versions of the same object.
#[derive(Default)]
pub(crate) struct MultipleReads {
    flags: HashMap<ObjectIdentifier, Result<PreparedFlags, Error>>,
    sources: HashMap<ObjectIdentifier, Result<PreparedValueSource, Error>>,
}

impl MultipleReads {
    /// Capture the specialized Value_Source report of `object` when `sub`
    /// selects it. Run for every reference before any [`Self::prepare`].
    pub fn capture_source(&mut self, object: &dyn BACnetObject, sub: &CovSubscription) {
        if sub.monitored_property != Some(PropertyIdentifier::VALUE_SOURCE)
            || !value_source::applies(object, PropertyIdentifier::VALUE_SOURCE)
        {
            return;
        }
        let oid = object.object_identifier();
        let flags = self
            .flags
            .entry(oid)
            .or_insert_with(|| PreparedFlags::read(object));
        if let Ok(flags) = flags {
            self.sources
                .entry(oid)
                .or_insert_with(|| PreparedValueSource::read(object, flags));
        }
    }

    /// Prepare `sub` from `object` when it reports against `baseline` (or
    /// unconditionally when `force`). `None` when unreportable or unreadable.
    pub fn prepare(
        &mut self,
        object: &dyn BACnetObject,
        sub: &CovSubscription,
        baseline: Option<&CovObservation>,
        force: bool,
    ) -> Option<PreparedReference> {
        let prepared = self.read(object, sub)?;
        (force || self.reports(&prepared, baseline)).then_some(prepared)
    }

    /// Read `sub`'s current values from `object`. Every object read of a
    /// preparation happens here; `None` when unreadable.
    pub fn read(
        &mut self,
        object: &dyn BACnetObject,
        sub: &CovSubscription,
    ) -> Option<PreparedReference> {
        let property_identifier = sub.monitored_property?;
        let oid = object.object_identifier();
        let flags = self
            .flags
            .entry(oid)
            .or_insert_with(|| PreparedFlags::read(object))
            .as_ref()
            .ok()?;
        if value_source::applies(object, property_identifier) {
            if sub.monitored_property_array_index.is_some() {
                return None;
            }
            let prepared = self.sources.get(&oid)?.as_ref().ok()?;
            return Some(PreparedReference {
                values: prepared
                    .values()
                    .into_iter()
                    .map(|value| COVNotificationValue {
                        property_identifier: value.property_identifier,
                        property_array_index: value.property_array_index,
                        value: value.value,
                        time_of_change: None,
                    })
                    .collect(),
                observation: prepared.observation.clone(),
                oid,
                criterion: None,
            });
        }
        let prepared = if property_identifier == PropertyIdentifier::STATUS_FLAGS {
            flags.selected(object, sub.monitored_property_array_index)
        } else {
            let captured = self
                .sources
                .get(&oid)
                .and_then(|source| source.as_ref().ok())
                .filter(|_| sub.monitored_property_array_index.is_none())
                .and_then(|source| source.value(property_identifier));
            captured
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| {
                    object.read_property(property_identifier, sub.monitored_property_array_index)
                })
                .and_then(|value| {
                    super::prepare::prepare_value(
                        object,
                        property_identifier,
                        sub.monitored_property_array_index,
                        sub.cov_increment,
                        &value,
                    )
                })
        }
        .ok()?;
        Some(PreparedReference {
            values: vec![COVNotificationValue {
                property_identifier,
                property_array_index: sub.monitored_property_array_index,
                value: prepared.encoded.clone(),
                time_of_change: None,
            }],
            observation: flags.observation(prepared.sample.clone()),
            oid,
            criterion: Some(prepared),
        })
    }

    /// Whether a read reference reports against `baseline`: its reporting
    /// criterion or a present Status_Flags change. Reads no object state.
    pub fn reports(&self, prepared: &PreparedReference, baseline: Option<&CovObservation>) -> bool {
        match &prepared.criterion {
            Some(value) => {
                value.reports(baseline.map(|o| o.sample()))
                    || prepared.observation.flags_changed(baseline)
            }
            None => self
                .sources
                .get(&prepared.oid)
                .and_then(|source| source.as_ref().ok())
                .is_some_and(|source| source.reports(baseline)),
        }
    }

    /// Encoded Status_Flags of an object already read by this preparation.
    pub fn encoded_flags(&self, oid: &ObjectIdentifier) -> Option<&[u8]> {
        self.flags.get(oid)?.as_ref().ok()?.encoded.as_deref()
    }

    /// `values` plus the object's Status_Flags companion, captured in the same
    /// read, unless the reference already conveys Status_Flags.
    pub fn with_flags_companion(
        &self,
        oid: &ObjectIdentifier,
        mut values: Vec<COVNotificationValue>,
    ) -> Vec<COVNotificationValue> {
        if values
            .iter()
            .all(|v| v.property_identifier != PropertyIdentifier::STATUS_FLAGS)
        {
            if let Some(encoded) = self.encoded_flags(oid) {
                values.push(COVNotificationValue {
                    property_identifier: PropertyIdentifier::STATUS_FLAGS,
                    property_array_index: None,
                    value: encoded.to_vec(),
                    time_of_change: None,
                });
            }
        }
        values
    }
}
