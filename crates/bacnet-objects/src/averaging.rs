//! Averaging (type 18) object per ASHRAE 135-2020 Clause 12.4.
//!
//! Computes running statistics (min, max, average) over sampled values from
//! a referenced object property.

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Averaging object (type 18).
///
/// Accumulates sample values and computes min/max/average statistics.
/// The `present_value` property reflects the current average.
pub struct AveragingObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    minimum_value: f32,
    maximum_value: f32,
    average_value: f32,
    attempted_samples: u32,
    valid_samples: u32,
    object_property_reference: Option<BACnetObjectPropertyReference>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AveragingObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::AVERAGING, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            minimum_value: f32::MAX,
            maximum_value: f32::MIN,
            average_value: 0.0,
            attempted_samples: 0,
            valid_samples: 0,
            object_property_reference: None,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Add a sample value, updating min/max/average and counts.
    pub fn add_sample(&mut self, value: f32) {
        self.attempted_samples += 1;
        self.valid_samples += 1;

        if value < self.minimum_value {
            self.minimum_value = value;
        }
        if value > self.maximum_value {
            self.maximum_value = value;
        }

        // Running average: avg = avg_prev + (value - avg_prev) / n
        self.average_value += (value - self.average_value) / self.valid_samples as f32;
        self.present_value = self.average_value;
    }

    /// Set the object property reference (the property being averaged).
    pub fn set_object_property_reference(
        &mut self,
        reference: Option<BACnetObjectPropertyReference>,
    ) {
        self.object_property_reference = reference;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for AveragingObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::AVERAGING.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::MINIMUM_VALUE => {
                if self.valid_samples == 0 {
                    Ok(PropertyValue::Real(0.0))
                } else {
                    Ok(PropertyValue::Real(self.minimum_value))
                }
            }
            p if p == PropertyIdentifier::MAXIMUM_VALUE => {
                if self.valid_samples == 0 {
                    Ok(PropertyValue::Real(0.0))
                } else {
                    Ok(PropertyValue::Real(self.maximum_value))
                }
            }
            p if p == PropertyIdentifier::AVERAGE_VALUE => {
                Ok(PropertyValue::Real(self.average_value))
            }
            p if p == PropertyIdentifier::ATTEMPTED_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.attempted_samples as u64))
            }
            p if p == PropertyIdentifier::VALID_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.valid_samples as u64))
            }
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                match &self.object_property_reference {
                    None => Ok(PropertyValue::Null),
                    Some(r) => {
                        let mut fields = vec![
                            PropertyValue::ObjectIdentifier(r.object_identifier),
                            PropertyValue::Unsigned(r.property_identifier as u64),
                        ];
                        if let Some(idx) = r.property_array_index {
                            fields.push(PropertyValue::Unsigned(idx as u64));
                        }
                        Ok(PropertyValue::List(fields))
                    }
                }
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        // Clause 12.5 Table 12-5 types Object_Property_Reference as
        // BACnetDeviceObjectPropertyReference, and the object text leaves
        // referencing an object in a DIFFERENT device explicit ("Optionally,
        // the object property to be sampled may exist in a different BACnet
        // device") — this implementation samples local objects only, so the
        // shared arm helper decodes with the local-only
        // BACnetObjectPropertyReference framing: a device-qualified member
        // [3] refuses INVALID_DATA_ENCODING instead of silently dropping the
        // device, and the flat form keeps its historical Unsigned members
        // (both Unsigned and Enumerated are accepted there; see
        // reference.rs).
        if property == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE {
            self.object_property_reference = crate::reference::decode_reference_write(
                &value,
                crate::reference::ReferenceFrame::Bare,
            )?;
            return Ok(());
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::MINIMUM_VALUE,
            PropertyIdentifier::MAXIMUM_VALUE,
            PropertyIdentifier::AVERAGE_VALUE,
            PropertyIdentifier::ATTEMPTED_SAMPLES,
            PropertyIdentifier::VALID_SAMPLES,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::EVENT_STATE,
        ];
        Cow::Borrowed(PROPS)
    }

    /// Mirror the `write_property` arms exactly (PICS truth invariant):
    /// Averaging accepts OBJECT_PROPERTY_REFERENCE plus the shared
    /// DESCRIPTION / OUT_OF_SERVICE routes. OBJECT_NAME is NOT advertised:
    /// unlike the historical default's blanket claim, no arm routes it (a
    /// network write falls through to WRITE_ACCESS_DENIED).
    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE
                | PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::OUT_OF_SERVICE
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn averaging_create() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_NAME, None)
                .unwrap(),
            PropertyValue::CharacterString("AVG-1".into())
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::AVERAGING.to_raw())
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn averaging_add_samples() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(10.0);
        avg.add_sample(20.0);
        avg.add_sample(30.0);

        assert_eq!(
            avg.read_property(PropertyIdentifier::ATTEMPTED_SAMPLES, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::VALID_SAMPLES, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
    }

    #[test]
    fn averaging_min_max() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(15.0);
        avg.add_sample(5.0);
        avg.add_sample(25.0);

        assert_eq!(
            avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(5.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(25.0)
        );
    }

    #[test]
    fn averaging_average_value() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(10.0);
        avg.add_sample(20.0);
        avg.add_sample(30.0);

        let val = avg
            .read_property(PropertyIdentifier::AVERAGE_VALUE, None)
            .unwrap();
        if let PropertyValue::Real(v) = val {
            assert!((v - 20.0).abs() < 0.001);
        } else {
            panic!("Expected Real");
        }

        // present_value should equal average_value
        let pv = avg
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, val);
    }

    #[test]
    fn averaging_no_samples_defaults() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        // Before any samples, min/max return 0.0
        assert_eq!(
            avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::AVERAGE_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn averaging_property_list() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        let props = avg.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::MINIMUM_VALUE));
        assert!(props.contains(&PropertyIdentifier::MAXIMUM_VALUE));
        assert!(props.contains(&PropertyIdentifier::AVERAGE_VALUE));
        assert!(props.contains(&PropertyIdentifier::ATTEMPTED_SAMPLES));
        assert!(props.contains(&PropertyIdentifier::VALID_SAMPLES));
        assert!(props.contains(&PropertyIdentifier::OBJECT_PROPERTY_REFERENCE));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn averaging_object_property_reference_default_null() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn averaging_set_object_property_reference() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(oid, pv_raw)));

        let val = avg
            .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ])
        );
    }

    #[test]
    fn averaging_write_object_property_reference() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();

        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ]),
            None,
        )
        .unwrap();

        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ])
        );
    }

    #[test]
    fn averaging_write_null_clears_reference() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));

        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();

        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn averaging_write_present_value_denied() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let result = avg.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn averaging_description_read_write() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString(String::new())
        );
        avg.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Zone temperature averaging".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Zone temperature averaging".into())
        );
    }

    #[test]
    fn averaging_single_sample() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(42.0);

        assert_eq!(
            avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::AVERAGE_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
    }

    // --- #182 adversary blocker: strict shared decode on the reference arm ---

    #[test]
    fn averaging_reference_write_accepts_exact_shapes_and_both_member_typings() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        for (label, members, expect_indexed) in [
            (
                "2-member Unsigned (historical flat form)",
                vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Unsigned(pv_raw as u64),
                ],
                None,
            ),
            (
                "2-member Enumerated (Loop-family flat form)",
                vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Enumerated(pv_raw),
                ],
                None,
            ),
            (
                "3-member indexed",
                vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Unsigned(pv_raw as u64),
                    PropertyValue::Unsigned(4),
                ],
                Some(4),
            ),
        ] {
            let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
            avg.write_property(
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                None,
                PropertyValue::List(members),
                None,
            )
            .unwrap_or_else(|e| panic!("{label}: must be accepted: {e:?}"));
            let mut expected = vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ];
            if let Some(idx) = expect_indexed {
                expected.push(PropertyValue::Unsigned(idx as u64));
            }
            assert_eq!(
                avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                    .unwrap(),
                PropertyValue::List(expected),
                "{label}: read-back fidelity"
            );
        }
    }

    #[test]
    fn averaging_reference_write_rejects_bad_shapes_and_preserves_state() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        let baseline = PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Unsigned(pv_raw as u64),
        ]);

        // A framed device-qualified write ([3] device-identifier): the
        // Clause 12.5 typing is BACnetDeviceObjectPropertyReference but the
        // remote-sample path is the standard's OPTIONAL branch, unmodeled
        // here — refused INVALID_DATA_ENCODING rather than silently
        // local-izing.
        let mut framed_device = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_object_property_reference(
            &mut framed_device,
            &BACnetObjectPropertyReference::new(oid, pv_raw),
        );
        bacnet_encoding::primitives::encode_ctx_object_id(
            &mut framed_device,
            3,
            &ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap(),
        );

        let cases: Vec<(PropertyValue, bacnet_types::enums::ErrorCode, &str)> = vec![
            (
                PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Unsigned(pv_raw as u64),
                    PropertyValue::Unsigned(2),
                    PropertyValue::Unsigned(9), // 4th member: silently dropped pre-fix
                ]),
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
                "4-member list",
            ),
            (
                PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Unsigned(pv_raw as u64),
                    PropertyValue::Real(2.0), // non-Unsigned 3rd: retyped to no-index pre-fix
                ]),
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
                "wrong-typed third member",
            ),
            (
                PropertyValue::List(vec![
                    PropertyValue::Unsigned(1),
                    PropertyValue::Unsigned(pv_raw as u64),
                ]),
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
                "non-ObjectIdentifier first member",
            ),
            (
                PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Unsigned(u64::MAX), // > 4 octets: `as u32` truncated pre-fix
                ]),
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
                "oversized Unsigned property member",
            ),
            (
                PropertyValue::ApplicationData(framed_device.to_vec()),
                bacnet_types::enums::ErrorCode::INVALID_DATA_ENCODING,
                "device-qualified framed reference",
            ),
        ];

        for (value, expected_code, label) in cases {
            let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
            avg.write_property(
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                None,
                baseline.clone(),
                None,
            )
            .unwrap();
            let err = avg
                .write_property(
                    PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                    None,
                    value,
                    None,
                )
                .expect_err(label);
            match err {
                Error::Protocol { class, code } => {
                    assert_eq!(
                        class,
                        bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                        "{label}: wrong class"
                    );
                    assert_eq!(code, expected_code.to_raw() as u32, "{label}: wrong code");
                }
                other => panic!("{label}: expected Property error, got {other:?}"),
            }
            assert_eq!(
                avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                    .unwrap(),
                baseline,
                "{label}: refused write must leave the stored reference untouched"
            );
        }
    }

    #[test]
    fn averaging_reference_write_accepts_the_framed_local_form() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        let mut framed = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_object_property_reference(
            &mut framed,
            &BACnetObjectPropertyReference::new_indexed(oid, pv_raw, 2),
        );
        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::ApplicationData(framed.to_vec()),
            None,
        )
        .unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
                PropertyValue::Unsigned(2),
            ])
        );
    }
}
