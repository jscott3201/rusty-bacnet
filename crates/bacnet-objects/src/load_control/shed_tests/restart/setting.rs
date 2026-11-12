//! The application's changes to a running object's configuration (#2497),
//! through its [`LocalInput`](crate::traits::LocalInput): a request under way
//! is looked at again, and what that moves is saved with the request.

use super::*;
use crate::load_control::LoadControlSetting;
use crate::traits::{LocalInput, LocalInputValue};

fn apply(lc: &mut LoadControlObject, setting: LoadControlSetting) -> Result<(), Error> {
    lc.apply_local_input(LocalInputValue::LoadControl(setting))
}

#[test]
fn a_setting_that_moves_a_request_is_saved_and_the_configuration_is_not() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    lc.wait_for_saves();
    assert_eq!(store.held().state, COMPLIANT);

    // The loads fall short once they hear of the new baseline.
    app.answer(ShedProgress::Shedding, 2, 1);
    app.take();
    apply(&mut lc, LoadControlSetting::FullDutyBaseline(Some(250.0))).unwrap();
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    assert!(matches!(&app.take()[..],
        [Told::Shed(r)] if r.full_duty_baseline == Some(250.0) && r.started));
    lc.wait_for_saves();
    assert_eq!(
        store.held().state,
        NON_COMPLIANT,
        "the state it moved was saved"
    );

    // The same for new levels, with the answer back.
    app.answer(ShedProgress::Compliant, 2, 2);
    apply(
        &mut lc,
        LoadControlSetting::ShedLevels(vec![(2, "a".into()), (5, "b".into())]),
    )
    .unwrap();
    assert_eq!(present_value(&lc), COMPLIANT);
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.shed_level == Some(0)));
    lc.wait_for_saves();
    assert_eq!(store.held().state, COMPLIANT);

    // A state description looks at nothing, and saves nothing.
    let saves = store.saves.load(std::sync::atomic::Ordering::SeqCst);
    apply(
        &mut lc,
        LoadControlSetting::StateDescription(Some("note".into())),
    )
    .unwrap();
    lc.wait_for_saves();
    assert!(app.take().is_empty());
    assert_eq!(store.saves.load(std::sync::atomic::Ordering::SeqCst), saves);
    assert_eq!(
        level(&lc, P::STATE_DESCRIPTION),
        PropertyValue::CharacterString("note".into())
    );

    // Refused as the setters refuse, and other inputs are not its own.
    assert!(
        apply(
            &mut lc,
            LoadControlSetting::ShedLevels(vec![(0, "x".into())])
        )
        .is_err()
    );
    assert!(apply(&mut lc, LoadControlSetting::FullDutyBaseline(Some(-1.0))).is_err());
    assert!(lc.apply_local_input(LocalInputValue::Pulses(1)).is_err());
}
