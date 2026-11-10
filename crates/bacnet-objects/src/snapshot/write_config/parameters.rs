//! The event algorithm parameters that end a
//! [`KIND_REPORTING_PARAMETERS`] body
//! (#2331). The parent module documents the layout.
//!
//! One table, `rows!`, names each parameter once: its field of
//! [`AlgorithmWriteConfig`], its bit of the presence mask and its property.
//! A parameter added later takes the next bit there, a line in [`kept`] for
//! each object type that keeps it, and its octets in [`encode`] and
//! [`Reader::parameters`]; the kind stays, and a build that doesn't keep
//! the bit for the object's type refuses the file as an unknown kind.

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bytes::BytesMut;

use super::{KIND_REPORTING_PARAMETERS, Reader, SnapshotError, SnapshotErrorKind};
use crate::write_config::AlgorithmWriteConfig;

/// Each parameter's bit of the presence mask. A bit keeps its meaning for
/// good: files hold it.
pub(crate) mod bit {
    /// Time_Delay: four octets.
    pub(crate) const TIME_DELAY: u32 = 1 << 0;
    /// Time_Delay_Normal: four octets.
    pub(crate) const TIME_DELAY_NORMAL: u32 = 1 << 1;
    /// High_Limit: the four octets of an IEEE 754 single.
    pub(crate) const HIGH_LIMIT: u32 = 1 << 2;
    /// Low_Limit, as High_Limit.
    pub(crate) const LOW_LIMIT: u32 = 1 << 3;
    /// Deadband, as High_Limit.
    pub(crate) const DEADBAND: u32 = 1 << 4;
    /// Limit_Enable: one octet, bit 0 the low limit and bit 1 the high.
    pub(crate) const LIMIT_ENABLE: u32 = 1 << 5;
}

/// What is wrong with a parameter outside [`kept`] for the object's type:
/// one a later build keeps for it, or one no build does.
const UNKEPT: &str = "holds an algorithm parameter this build doesn't keep for this object type";

/// The parameter table: `field => BIT / PROPERTY` for each.
macro_rules! rows {
    ($($field:ident => $bit:ident / $property:ident),* $(,)?) => {
        /// The presence mask of `config`: the bits of the parameters it
        /// holds.
        pub(crate) fn mask(config: &AlgorithmWriteConfig) -> u32 {
            let mut mask = 0;
            $(if config.$field.is_some() {
                mask |= bit::$bit;
            })*
            mask
        }

        /// The parameters of `config` whose bits are in `mask`.
        // event-reporting: only the reporting store picks rows.
        #[cfg(feature = "event-reporting")]
        pub(crate) fn only(config: &AlgorithmWriteConfig, mask: u32) -> AlgorithmWriteConfig {
            AlgorithmWriteConfig {
                $($field: config.$field.filter(|_| mask & bit::$bit != 0),)*
            }
        }

        /// Put the parameters `saved` holds over those of `onto`.
        #[cfg(feature = "event-reporting")]
        pub(crate) fn overlay(onto: &mut AlgorithmWriteConfig, saved: &AlgorithmWriteConfig) {
            $(if saved.$field.is_some() {
                onto.$field = saved.$field;
            })*
        }

        /// The mask bit of `property`, if it is a parameter this build
        /// keeps for some object type.
        pub(crate) fn bit_of(property: P) -> Option<u32> {
            match property {
                $(P::$property => Some(bit::$bit),)*
                _ => None,
            }
        }
    };
}

rows! {
    time_delay => TIME_DELAY / TIME_DELAY,
    time_delay_normal => TIME_DELAY_NORMAL / TIME_DELAY_NORMAL,
    high_limit => HIGH_LIMIT / HIGH_LIMIT,
    low_limit => LOW_LIMIT / LOW_LIMIT,
    deadband => DEADBAND / DEADBAND,
    limit_enable => LIMIT_ENABLE / LIMIT_ENABLE,
}

/// The parameters objects of `object_type` keep, as mask bits: the two time
/// delays and the four OUT_OF_RANGE parameters on the analog Input, Output
/// and Value objects. No bit for a type that keeps none.
pub(crate) fn kept(object_type: ObjectType) -> u32 {
    match object_type {
        ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE => {
            bit::TIME_DELAY
                | bit::TIME_DELAY_NORMAL
                | bit::HIGH_LIMIT
                | bit::LOW_LIMIT
                | bit::DEADBAND
                | bit::LIMIT_ENABLE
        }
        _ => 0,
    }
}

/// Whether objects of `object_type` keep a written `property` as an event
/// algorithm parameter in a
/// [`KIND_REPORTING_PARAMETERS`] file
/// (#2331): Time_Delay, Time_Delay_Normal, High_Limit, Low_Limit, Deadband
/// and Limit_Enable on the analog Input, Output and Value objects. A file
/// holding a parameter the object's type doesn't keep here is refused as an
/// unknown kind: a later build may keep it.
pub fn keeps_parameter(object_type: ObjectType, property: P) -> bool {
    bit_of(property).is_some_and(|bit| kept(object_type) & bit != 0)
}

/// Why `config` holds what no write to an object of `object_type` leaves: a
/// parameter the type doesn't keep, a limit or deadband that isn't finite,
/// a negative deadband, or Limit_Enable bits past its two. The object
/// refuses each on the wire, so storage holding one is refused too.
pub(crate) fn invalid(
    object_type: ObjectType,
    config: &AlgorithmWriteConfig,
) -> Option<&'static str> {
    if mask(config) & !kept(object_type) != 0 {
        return Some(UNKEPT);
    }
    let limits = [config.high_limit, config.low_limit, config.deadband];
    if limits.iter().flatten().any(|limit| !limit.is_finite()) {
        return Some("holds a limit or deadband that isn't finite");
    }
    if config.deadband.is_some_and(|deadband| deadband < 0.0) {
        return Some("holds a negative Deadband");
    }
    if config
        .limit_enable
        .is_some_and(|enable| LimitEnable::from_bits(enable.bits()).is_none())
    {
        return Some("holds unknown Limit_Enable bits");
    }
    None
}

/// Append `config`'s parameters for an object of `object_type`: the mask,
/// then each parameter it holds in bit order. Refuses one [`invalid`]
/// names.
pub(super) fn encode(
    body: &mut BytesMut,
    object_type: ObjectType,
    config: &AlgorithmWriteConfig,
) -> Result<(), SnapshotError> {
    if let Some(reason) = invalid(object_type, config) {
        return Err(SnapshotError::out_of_range(reason));
    }
    body.extend_from_slice(&mask(config).to_be_bytes());
    let unsigned = [config.time_delay, config.time_delay_normal];
    for value in unsigned.into_iter().flatten() {
        body.extend_from_slice(&value.to_be_bytes());
    }
    let reals = [config.high_limit, config.low_limit, config.deadband];
    for value in reals.into_iter().flatten() {
        body.extend_from_slice(&value.to_bits().to_be_bytes());
    }
    if let Some(enable) = config.limit_enable {
        body.extend_from_slice(&[enable.bits()]);
    }
    Ok(())
}

impl Reader<'_> {
    /// The parameters after a reporting body, for an object of
    /// `object_type`. A bit outside [`kept`] for that type, whether this
    /// build knows it for another type or not at all, is a newer build's
    /// sound file ([`UnknownKind`](SnapshotErrorKind::UnknownKind)), refused
    /// before a row is read, and never damage: a backend with two copies
    /// mustn't pass it over for an older one. No bit at all, or a kept
    /// parameter holding what [`invalid`] names, is damage.
    pub(super) fn parameters(
        &mut self,
        object_type: ObjectType,
    ) -> Result<AlgorithmWriteConfig, SnapshotError> {
        let mask = self.u32()?;
        if mask & !kept(object_type) != 0 {
            return Err(SnapshotError::new(
                SnapshotErrorKind::UnknownKind(KIND_REPORTING_PARAMETERS),
                UNKEPT,
            ));
        }
        if mask == 0 {
            return Err(SnapshotError::damaged(
                "is of the kind with algorithm parameters and holds none",
            ));
        }
        let mut config = AlgorithmWriteConfig::default();
        let has = |bit: u32| mask & bit != 0;
        if has(bit::TIME_DELAY) {
            config.time_delay = Some(self.u32()?);
        }
        if has(bit::TIME_DELAY_NORMAL) {
            config.time_delay_normal = Some(self.u32()?);
        }
        if has(bit::HIGH_LIMIT) {
            config.high_limit = Some(f32::from_bits(self.u32()?));
        }
        if has(bit::LOW_LIMIT) {
            config.low_limit = Some(f32::from_bits(self.u32()?));
        }
        if has(bit::DEADBAND) {
            config.deadband = Some(f32::from_bits(self.u32()?));
        }
        if has(bit::LIMIT_ENABLE) {
            config.limit_enable = Some(LimitEnable::from_bits_retain(self.u8()?));
        }
        match invalid(object_type, &config) {
            Some(reason) => Err(SnapshotError::damaged(reason)),
            None => Ok(config),
        }
    }
}
