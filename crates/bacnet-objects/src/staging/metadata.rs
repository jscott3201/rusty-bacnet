use bacnet_types::enums::PropertyIdentifier;

use crate::property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability};

// Table 12-80 codes only Present_Value W. Stages, Target_References,
// Out_Of_Service, Priority_For_Writing and Min_Pres_Value are R there; this
// device takes writes to them, which their capability says (#2174).
pub(super) const STAGING_PROPERTY_METADATA: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_NAME,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::DESCRIPTION,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRESENT_VALUE,
        PropertyConformance::RequiredWrite,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRESENT_STAGE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::STAGES,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::TARGET_REFERENCES,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::STATUS_FLAGS,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_STATE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::RELIABILITY,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::WhenOutOfService,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::UNITS,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::MIN_PRES_VALUE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::MAX_PRES_VALUE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    // Table 12-80 footnote 3: present because the object supports COV.
    PropertyMetadata::new(
        PropertyIdentifier::COV_INCREMENT,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use PropertyIdentifier as P;

    #[test]
    fn staging_rows_carry_the_table_codes_and_their_write_routes() {
        // Table 12-80 codes only Present_Value W; this device writes these
        // R rows too, which their capability carries (#2174).
        let written_r_rows = [
            P::STAGES,
            P::TARGET_REFERENCES,
            P::OUT_OF_SERVICE,
            P::PRIORITY_FOR_WRITING,
            P::MIN_PRES_VALUE,
        ];
        for row in STAGING_PROPERTY_METADATA {
            let p = row.property_identifier;
            let expected = match p {
                P::PRESENT_VALUE => PropertyConformance::RequiredWrite,
                P::DESCRIPTION | P::COV_INCREMENT => PropertyConformance::Optional,
                _ => PropertyConformance::RequiredRead,
            };
            assert_eq!(row.conformance, expected, "{p:?}");
            assert_eq!(row.presence_condition, None, "{p:?}");
            if written_r_rows.contains(&p) {
                assert_eq!(
                    row.write_capability,
                    PropertyWriteCapability::Always,
                    "{p:?}"
                );
            }
        }
        for p in written_r_rows {
            assert!(
                STAGING_PROPERTY_METADATA
                    .iter()
                    .any(|row| row.property_identifier == p)
            );
        }
    }
}
