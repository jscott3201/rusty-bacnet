//! Credential reasons and coherent validity are prerequisites to every grant.
use super::authorization_test_support::*;
use super::*;
use crate::clock::{ClockFrame, ClockReader};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

macro_rules! reason_case {
    ($name:ident, $reason:ident, $event:ident) => {
        #[test]
        fn $name() {
            let mut db = database(None);
            reasons(&mut db, 1, &[Reason::$reason]);
            denied(&eval(&db, request()), AccessEvent::$event);
        }
    };
}
reason_case!(disabled_generic, DISABLED, DENIED_CREDENTIAL_DISABLED);
reason_case!(
    disabled_provisioning,
    DISABLED_NEEDS_PROVISIONING,
    DENIED_CREDENTIAL_NOT_PROVISIONED
);
reason_case!(
    disabled_unassigned,
    DISABLED_UNASSIGNED,
    DENIED_CREDENTIAL_UNASSIGNED
);
reason_case!(
    disabled_lockout,
    DISABLED_LOCKOUT,
    DENIED_CREDENTIAL_LOCKOUT
);
reason_case!(
    disabled_max_days,
    DISABLED_MAX_DAYS,
    DENIED_CREDENTIAL_MAX_DAYS
);
reason_case!(
    disabled_max_uses,
    DISABLED_MAX_USES,
    DENIED_CREDENTIAL_MAX_USES
);
reason_case!(
    disabled_inactivity,
    DISABLED_INACTIVITY,
    DENIED_CREDENTIAL_INACTIVITY
);
reason_case!(
    disabled_manual,
    DISABLED_MANUAL,
    DENIED_CREDENTIAL_MANUAL_DISABLE
);

#[test]
fn not_yet_active_has_its_specific_denial() {
    let mut db = database(None);
    value(&mut db, credential(1), P::ACTIVATION_TIME, date_time(10));
    reasons(&mut db, 1, &[Reason::DISABLED_NOT_YET_ACTIVE]);
    let mut req = request();
    req.wall_time = Some(frame(9));
    denied(
        &eval(&db, req),
        AccessEvent::DENIED_CREDENTIAL_NOT_YET_ACTIVE,
    );
}
#[test]
fn expired_has_its_specific_denial() {
    let mut db = database(None);
    value(&mut db, credential(1), P::EXPIRATION_TIME, date_time(12));
    reasons(&mut db, 1, &[Reason::DISABLED_EXPIRED]);
    let mut req = request();
    req.wall_time = Some(frame(13));
    denied(&eval(&db, req), AccessEvent::DENIED_CREDENTIAL_EXPIRED);
}
#[test]
fn reason_priority_is_explicit_and_reversible() {
    let mut db = database(None);
    reasons(
        &mut db,
        1,
        &[Reason::DISABLED_MANUAL, Reason::DISABLED_LOCKOUT],
    );
    denied(
        &eval(&db, request()),
        AccessEvent::DENIED_CREDENTIAL_LOCKOUT,
    );
    let mut req = request();
    let mut priorities = REASONS;
    priorities.reverse();
    req.disable_reason_priority = DisableReasonPriority::new(priorities).unwrap();
    denied(
        &eval(&db, req),
        AccessEvent::DENIED_CREDENTIAL_MANUAL_DISABLE,
    );
}
#[test]
fn complete_order_and_named_reason_priority_cannot_omit_members() {
    for i in 0..7 {
        let mut checks = CHECKS;
        checks[i] = CHECKS[(i + 1) % 7];
        assert!(AuthorizationOrder::new(checks).is_err());
    }
    for i in 0..10 {
        let mut reasons = REASONS;
        reasons[i] = REASONS[(i + 1) % 10];
        assert!(DisableReasonPriority::new(reasons).is_err());
        reasons[i] = Reason::from_raw(64);
        assert!(DisableReasonPriority::new(reasons).is_err());
    }
}
#[test]
fn unsupported_and_duplicate_reasons_never_disappear_beside_known_denials() {
    for reasons_list in [
        vec![Reason::from_raw(64)],
        vec![Reason::DISABLED, Reason::from_raw(10)],
        vec![Reason::DISABLED_MANUAL, Reason::DISABLED_MANUAL],
    ] {
        let mut db = database(None);
        reasons(&mut db, 1, &reasons_list);
        unresolved(&eval(&db, request()));
    }
}
#[test]
fn contradictory_or_unknown_status_is_unresolved() {
    for (status, reasons_list) in [
        (1, vec![Reason::DISABLED_MANUAL]),
        (0, vec![]),
        (64, vec![]),
    ] {
        let mut db = database(None);
        reasons(&mut db, 1, &reasons_list);
        value(
            &mut db,
            credential(1),
            P::CREDENTIAL_STATUS,
            PropertyValue::Enumerated(status),
        );
        unresolved(&eval(&db, request()));
    }
}
#[test]
fn malformed_credential_rows_never_grant() {
    for property in [
        P::CREDENTIAL_STATUS,
        P::REASON_FOR_DISABLE,
        P::ACTIVATION_TIME,
        P::EXPIRATION_TIME,
    ] {
        let mut db = database(None);
        value(&mut db, credential(1), property, PropertyValue::Null);
        unresolved(&eval(&db, request()));
    }
    let mut db = database(None);
    value(
        &mut db,
        credential(1),
        P::REASON_FOR_DISABLE,
        PropertyValue::List(vec![PropertyValue::Boolean(false)]),
    );
    unresolved(&eval(&db, request()));
}
#[test]
fn matching_fixed_clocks_respect_both_inclusive_window_edges() {
    for (hour, event) in [
        (9, Some(AccessEvent::DENIED_CREDENTIAL_NOT_YET_ACTIVE)),
        (10, None),
        (11, None),
        (12, None),
        (13, Some(AccessEvent::DENIED_CREDENTIAL_EXPIRED)),
    ] {
        let mut db = database(None);
        install_window(&mut db, 1);
        db.set_clock_reader(Some(Arc::new(FixedClock(Some(frame(hour))))));
        let before = snapshot(&db);
        let mut req = request();
        req.wall_time = Some(frame(hour));
        let result = eval(&db, req);
        if let Some(event) = event {
            denied(&result, event);
        } else {
            granted(&result);
        }
        assert_eq!(snapshot(&db), before);
        assert_eq!(eval(&db, req), result);
    }
}
#[test]
fn open_limits_need_no_usable_wall_time() {
    let db = database(None);
    granted(&eval(&db, request()));
    let mut req = request();
    let mut invalid = frame(10);
    invalid.local_time.hour = 255;
    req.wall_time = Some(invalid);
    granted(&eval(&db, req));
}
#[test]
fn finite_limits_need_valid_coherent_supplied_time() {
    let mut db = database(None);
    install_window(&mut db, 1);
    db.set_clock_reader(Some(Arc::new(FixedClock(Some(frame(11))))));
    unresolved(&eval(&db, request()));
    let mut invalid = frame(11);
    invalid.local_date.day_of_week = 1;
    let mut req = request();
    req.wall_time = Some(invalid);
    unresolved(&eval(&db, req));
    for supplied in [9, 13] {
        req.wall_time = Some(frame(supplied));
        unresolved(&eval(&db, req));
    }
    for read_hour in [9, 13] {
        db.set_clock_reader(Some(Arc::new(FixedClock(Some(frame(read_hour))))));
        req.wall_time = Some(frame(11));
        unresolved(&eval(&db, req));
    }
}
#[test]
fn missing_or_invalid_getter_time_cannot_repair_supplied_expiration() {
    for getter in [None, {
        let mut f = frame(13);
        f.local_time.hour = 255;
        Some(f)
    }] {
        let mut db = database(None);
        install_window(&mut db, 1);
        db.set_clock_reader(Some(Arc::new(FixedClock(getter))));
        let mut req = request();
        req.wall_time = Some(frame(13));
        unresolved(&eval(&db, req));
    }
}
struct ScriptedClock {
    frames: [ClockFrame; 2],
    reads: AtomicUsize,
}
impl ClockReader for ScriptedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        let index = self.reads.fetch_add(1, Ordering::SeqCst);
        Some(self.frames[index.min(1)])
    }
}
#[test]
fn changing_clock_is_read_once_per_status_row_without_retrying_into_success() {
    for (first, second) in [(9, 11), (13, 11), (11, 13)] {
        let mut db = database(None);
        install_window(&mut db, 1);
        let clock = Arc::new(ScriptedClock {
            frames: [frame(first), frame(second)],
            reads: AtomicUsize::new(0),
        });
        db.set_clock_reader(Some(clock.clone()));
        clock.reads.store(0, Ordering::SeqCst);
        let mut req = request();
        req.wall_time = Some(frame(11));
        let result = eval(&db, req);
        unresolved(&result);
        assert_eq!(
            clock.reads.load(Ordering::SeqCst),
            2,
            "no retries or unrelated clock reads"
        );
        assert_eq!(result.credentials.len(), 1);
    }
}
#[test]
fn no_exemption_bypasses_activity_or_validity() {
    for raw in 0..=6 {
        let mut db = database(None);
        reasons(&mut db, 1, &[Reason::DISABLED_LOCKOUT]);
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        denied(
            &eval(&db, request()),
            AccessEvent::DENIED_CREDENTIAL_LOCKOUT,
        );
        value(&mut db, credential(1), P::EXPIRATION_TIME, date_time(12));
        reasons(&mut db, 1, &[Reason::DISABLED_EXPIRED]);
        let mut req = request();
        req.wall_time = Some(frame(13));
        denied(&eval(&db, req), AccessEvent::DENIED_CREDENTIAL_EXPIRED);
    }
}
#[test]
fn native_manual_disable_is_preserved() {
    let mut db = database(None);
    db.get_mut(&credential(1))
        .unwrap()
        .write_property(
            P::CREDENTIAL_DISABLE,
            None,
            PropertyValue::Enumerated(
                bacnet_types::enums::AccessCredentialDisable::DISABLE_MANUAL.to_raw(),
            ),
            None,
        )
        .unwrap();
    let before = snapshot(&db);
    denied(
        &eval(&db, request()),
        AccessEvent::DENIED_CREDENTIAL_MANUAL_DISABLE,
    );
    assert_eq!(snapshot(&db), before);
}
