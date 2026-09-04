use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

#[test]
fn python_staging_boundary_maps_typed_tuples_and_local_references_exactly() {
    let target = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 7).unwrap();
    let config = staging_config(
        5.0,
        0.0,
        62,
        8,
        vec![(10.0, vec![false], 1.0), (20.0, vec![true], 1.0)],
        vec![PyObjectIdentifier::from_rust(target)],
        Some(vec!["Off".into(), "On".into()]),
    );

    assert_eq!(config.present_value, 5.0);
    assert_eq!(config.stages[1].values, vec![true]);
    assert_eq!(config.target_references[0].device_identifier, None);
    assert_eq!(config.target_references[0].object_identifier, target);
    assert_eq!(config.stage_names.unwrap(), vec!["Off", "On"]);
}
