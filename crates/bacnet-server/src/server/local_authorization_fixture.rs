//! Test-owned explicit contexts and independent wire expectations.
use super::*;
pub(super) use bacnet_types::enums::{
    AccessPassbackMode as Passback, AuthorizationExemption as Exemption,
    AuthorizationMode as AuthMode, PropertyIdentifier as P,
};

pub(super) fn card_id(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, instance).unwrap()
}
pub(super) fn configured(
    db: &mut ObjectDatabase,
    mode: AuthMode,
    accompaniment: bool,
    exempt: bool,
) {
    let mut point = AccessPointObject::new(1, "point").unwrap();
    point
        .set_supported_authorization_modes([AuthMode::AUTHORIZE, mode])
        .unwrap();
    point
        .write_property(
            P::AUTHORIZATION_MODE,
            None,
            PropertyValue::Enumerated(mode.to_raw()),
            None,
        )
        .unwrap();
    point
        .set_access_event(access_event(AccessEvent::GRANTED, 1, 7))
        .unwrap();
    db.add(Box::new(point)).unwrap();
    for n in 1..=2 {
        let rights_id = ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, n).unwrap();
        let mut rights = AccessRightsObject::new(n, format!("rights-{n}")).unwrap();
        rights
            .set_positive_access_rules([BACnetAccessRule::new(
                None,
                Some(identifier(ObjectType::ACCESS_POINT).into()),
                true,
            )])
            .unwrap();
        if n == 1 && accompaniment {
            rights.set_accompaniment(Some(card_id(2).into())).unwrap();
        }
        db.add(Box::new(rights)).unwrap();
        let mut card = AccessCredentialObject::new(n, format!("card-{n}")).unwrap();
        card.set_assigned_access_rights(vec![BACnetAssignedAccessRights {
            assigned_access_rights: rights_id.into(),
            enable: true,
        }])
        .unwrap();
        if exempt && n == 1 {
            card.set_authorization_exemptions(Some(vec![Exemption::PASSBACK]))
                .unwrap();
        }
        db.add(Box::new(card)).unwrap();
    }
    let mut zone = AccessZoneObject::new(1, "zone").unwrap();
    zone.set_entry_points(vec![identifier(ObjectType::ACCESS_POINT)])
        .unwrap();
    zone.set_exit_points(vec![identifier(ObjectType::ACCESS_POINT)])
        .unwrap();
    zone.set_occupancy_count_enable(true);
    zone.set_occupancy_count(10);
    zone.set_occupancy_limits(1, 10).unwrap();
    db.add(Box::new(zone)).unwrap();
}
pub(super) fn companion() -> CredentialPresentation {
    CredentialPresentation {
        credential: card_id(2),
        sequence: 11,
        at: Duration::from_secs(101),
        ..request(false).original
    }
}
pub(super) fn soft(req: &mut AuthorizationRequest<'_>) {
    req.passback = AuthorizationPassback::Zone {
        zone: identifier(ObjectType::ACCESS_ZONE),
        fallback_mode: AuthorizationFact::Known(Passback::SOFT_PASSBACK),
        fallback_timeout_minutes: None,
        observation: AuthorizationFact::Known(AuthorizationPassbackObservation {
            presentation: req.original,
            zone: identifier(ObjectType::ACCESS_ZONE),
            violation: true,
            last_successful_entry: None,
        }),
    };
}
pub(super) fn phase(req: &mut AuthorizationRequest<'_>, verdict: AuthorizationVerdict) {
    req.external_phase = AuthorizationExternalPhase::Observed {
        presentation: req.original,
        started: req.original.at,
        fallback_period: AuthorizationFact::Known(Duration::from_secs(10)),
        deadline: AccompanimentDeadline::Inclusive,
        verdict,
    };
}
pub(super) fn expected_values(
    event: AccessEvent,
    tag: u64,
    hundredths: u8,
    credential: u8,
) -> Values {
    let mut timestamp = stamp_bytes(20);
    timestamp[10] = hundredths;
    vec![
        (P::ACCESS_EVENT, enumerated(event.to_raw())),
        (SF, normal()),
        (P::ACCESS_EVENT_TAG, unsigned(tag)),
        (P::ACCESS_EVENT_TIME, timestamp),
        (P::ACCESS_EVENT_CREDENTIAL, credential_bytes(credential)),
        (
            P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
            undefined_factor_bytes(),
        ),
    ]
}
pub(super) async fn snapshot(h: &Harness) -> Vec<PropertyValue> {
    let db = h.server.database().read().await;
    let point = db.get(&identifier(ObjectType::ACCESS_POINT)).unwrap();
    [
        P::ACCESS_EVENT,
        P::ACCESS_EVENT_TAG,
        P::ACCESS_EVENT_TIME,
        P::ACCESS_EVENT_CREDENTIAL,
        P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
    ]
    .map(|p| point.read_property(p, None).unwrap())
    .to_vec()
}
