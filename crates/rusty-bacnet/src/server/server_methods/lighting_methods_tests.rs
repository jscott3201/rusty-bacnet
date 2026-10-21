use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier as P;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty-bacnet-py-color-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pv(object: &dyn BACnetObject) -> PropertyValue {
    object.read_property(P::PRESENT_VALUE, None).unwrap()
}

/// The keywords' order (#1526): a value storage brought back wins over
/// `present_value`, which applies only while storage holds nothing.
#[test]
fn python_storage_path_keeps_the_saved_value_over_present_value() {
    let dir = scratch("xy");
    let path = dir.join("color-1");
    let output = |present: (f32, f32)| ColorOutput {
        present_value: Some(BACnetXyColor::new(present.0, present.1)),
        default: Some(BACnetXyColor::new(0.0, 0.0)),
        storage_path: path.to_str(),
    };
    let mut first = color(1, "C", output((0.25, 0.5))).unwrap();
    assert!(!first.output_restored());
    first
        .set_present_value(BACnetXyColor::new(0.4, 0.4))
        .unwrap();
    drop(first);

    let second = color(1, "C", output((0.25, 0.5))).unwrap();
    assert!(second.output_restored());
    assert_eq!(
        pv(&second),
        PropertyValue::List(vec![PropertyValue::Real(0.4), PropertyValue::Real(0.4)])
    );
    assert_eq!(second.default_color(), BACnetXyColor::new(0.0, 0.0));
    drop(second);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn python_color_temperature_keywords_clamp_and_keep_the_saved_value() {
    let dir = scratch("kelvin");
    let path = dir.join("ct-1");
    let output = |present: u32| ColorOutput {
        present_value: Some(present),
        default: Some(9_000),
        storage_path: path.to_str(),
    };
    let first = color_temperature(1, "CT", Some((2_000, 6_500)), output(2_700)).unwrap();
    assert_eq!(first.default_color_temperature(), 6_500);
    assert_eq!(pv(&first), PropertyValue::Unsigned(2_700));
    drop(first);

    let second = color_temperature(1, "CT", Some((3_000, 6_500)), output(5_000)).unwrap();
    assert!(second.output_restored());
    // The saved 2700 K comes back, clamped to the new minimum.
    assert_eq!(pv(&second), PropertyValue::Unsigned(3_000));
    drop(second);
    let _ = std::fs::remove_dir_all(&dir);
}
