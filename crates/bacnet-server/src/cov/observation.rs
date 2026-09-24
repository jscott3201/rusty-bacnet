//! One immutable delivered selected-value/flags observation per accepted reference.
use super::CovSample;
use bacnet_types::{error::Error, primitives::PropertyValue};

/// A bounded selected sample and its declared optional four-bit Status_Flags.
/// No delivered observation is represented by the subscription's outer `None`;
/// an observation with absent flags is a distinct successful delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovObservation {
    sample: CovSample,
    flags: Option<u8>,
}
impl CovObservation {
    /// Pair a validated selected sample with absent or canonical present flags.
    /// Present flags must be a one-byte BitString with four unused zero low bits.
    pub fn new(sample: CovSample, flags: Option<&PropertyValue>) -> Result<Self, Error> {
        Ok(Self {
            sample,
            flags: flags.map(validate_flags).transpose()?,
        })
    }
    /// The immutable bounded selected value.
    pub fn sample(&self) -> &CovSample {
        &self.sample
    }
    /// The four used status bits, or declared absence at delivery.
    pub fn status_flags(&self) -> Option<u8> {
        self.flags
    }
    /// Only a present changed value is a status trigger; disappearance is not.
    pub(crate) fn flags_changed(&self, previous: Option<&Self>) -> bool {
        self.flags.is_some() && previous.map(|p| p.flags) != Some(self.flags)
    }
}
pub(crate) fn validate_flags(value: &PropertyValue) -> Result<u8, Error> {
    match value {
        PropertyValue::BitString {
            unused_bits: 4,
            data,
        } if data.len() == 1 && data[0] & 0x0f == 0 => Ok(data[0] >> 4),
        _ => Err(Error::Encoding(
            "COV Status_Flags must be a canonical four-bit BitString".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cov_status_observation_validation_and_presence_semantics() {
        let sample = CovSample::new(&PropertyValue::Real(1.0)).unwrap();
        let absent = CovObservation::new(sample.clone(), None).unwrap();
        let flags = PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x80],
        };
        let present = CovObservation::new(sample.clone(), Some(&flags)).unwrap();
        assert!(present.flags_changed(None));
        assert!(present.flags_changed(Some(&absent)));
        assert!(!present.flags_changed(Some(&present)));
        assert!(!absent.flags_changed(Some(&present)));
        assert_eq!(present.status_flags(), Some(8));
        assert_eq!(absent.status_flags(), None);
        for invalid in [
            PropertyValue::Null,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0],
            },
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![],
            },
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0, 0],
            },
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0x81],
            },
        ] {
            assert!(CovObservation::new(sample.clone(), Some(&invalid)).is_err());
        }
    }
}
