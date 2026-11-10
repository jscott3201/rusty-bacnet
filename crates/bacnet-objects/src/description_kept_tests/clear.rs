//! `clear_written_description` (#2364), on the eight types that have no
//! clear covering all they keep: Notification Class, Notification
//! Forwarder, Access Rights, Device, Lighting Output, Binary Lighting
//! Output, Color and Color Temperature. It drops the saved Description, so
//! the file goes back to the version without one and the setter applies
//! again; a save that fails keeps it; and it saves nothing when there is
//! nothing to drop.

use super::*;

/// The types with the clear: every case but the three logs.
fn clearing(saved: Option<&str>) -> Vec<Box<dyn Kept>> {
    let cases: Vec<_> = cases(saved)
        .into_iter()
        .filter(|case| case.clears())
        .collect();
    assert_eq!(cases.len(), 8);
    cases
}

fn clear(object: &mut dyn KeptObject) -> Result<(), Error> {
    object.clear().expect("a type with the clear")
}

/// The clear saves before it returns: storage then holds no Description and
/// the file is the older version again. The text served stays until the
/// setter, which applies again, or a write changes it, and a rebuilt object
/// starts without it.
#[test]
fn the_clear_drops_the_saved_description_and_the_setter_applies_again() {
    for case in clearing(None) {
        let (property, value) = case.other();
        let mut object = case.open();
        write(object.object_mut(), property, value).unwrap();
        let plain = case.saved_tag().unwrap();
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        let described = case.saved_tag().unwrap();
        assert_ne!(described, plain, "{property:?}");
        assert_eq!(described[..6], plain[..6]);
        object.configure("configured");
        assert_eq!(description(object.object()), text("lobby"));

        let saves = case.saves();
        clear(object.as_mut()).unwrap();
        assert_eq!(case.saves(), saves + 1, "{property:?}");
        assert_eq!(case.saved_description(), None);
        assert_eq!(case.saved_tag().unwrap(), plain);
        assert_eq!(description(object.object()), text("lobby"));
        if case.has_setter() {
            object.configure("configured");
            assert_eq!(description(object.object()), text("configured"));
        }
        // The other kept property is still saved.
        let other = object.object().read_property(property, None).unwrap();
        drop(object);
        let rebuilt = case.open();
        assert_eq!(description(rebuilt.object()), text(""));
        assert_eq!(
            rebuilt.object().read_property(property, None).unwrap(),
            other
        );
    }
}

/// A clear whose save fails returns storage's error and keeps the written
/// Description: saved, served, and still winning over the setter.
#[test]
fn a_clear_whose_save_fails_keeps_the_description() {
    for case in clearing(Some("lobby")) {
        let mut object = case.open();
        case.fail(true);
        let result = clear(object.as_mut());
        assert!(
            matches!(&result, Err(Error::Encoding(reason)) if reason == "storage unavailable"),
            "{:?}: {result:?}",
            case.other().0
        );
        object.configure("configured");
        assert_eq!(description(object.object()), text("lobby"));
        case.fail(false);
        drop(object);
        assert_eq!(case.saved_description().as_deref(), Some("lobby"));
        assert_eq!(description(case.open().object()), text("lobby"));
    }
}

/// With no saved Description the clear saves nothing; nor does it without
/// storage. A write after a clear is kept again.
#[test]
fn a_clear_with_nothing_saved_does_nothing() {
    for case in clearing(None) {
        let mut object = case.open();
        clear(object.as_mut()).unwrap();
        assert_eq!(case.saves(), 0);
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        clear(object.as_mut()).unwrap();
        clear(object.as_mut()).unwrap();
        assert_eq!(case.saves(), 2);
        write(object.object_mut(), P::DESCRIPTION, text("hall")).unwrap();
        assert_eq!(case.saved_description().as_deref(), Some("hall"));
    }
    let mut class = NotificationClass::new(1, "NC").unwrap();
    class.clear_written_description().unwrap();
    let mut device = crate::device::DeviceObject::new(Default::default()).unwrap();
    device.clear_written_description().unwrap();
    let mut color = crate::color::ColorObject::new(1, "CLR").unwrap();
    color.clear_written_description().unwrap();
}

/// A write staged before a clear gives way to it: the clear's save lands
/// after the staged one, and the request's write then saves where it is, on
/// top of the cleared state.
#[test]
fn a_clear_supersedes_a_staged_write() {
    for case in clearing(Some("lobby")) {
        let mut object = case.open();
        let Some(staging) = object.staging() else {
            continue;
        };
        let StageStep::Staged(wait) =
            staging.stage_writes(&[pending(P::DESCRIPTION, text("hall"))])
        else {
            panic!("expected a staged write");
        };
        wait.block();
        clear(object.as_mut()).unwrap();
        assert_eq!(case.saved_description(), None, "{:?}", case.other().0);
        // Dropping the staged write may put the served state back first.
        write(object.object_mut(), P::DESCRIPTION, text("hall")).unwrap();
        object
            .staging()
            .expect("lent above")
            .release_staged_write(&wait);
        assert_eq!(description(object.object()), text("hall"));
        drop(object);
        assert_eq!(case.saved_description().as_deref(), Some("hall"));
    }
}
