//! SubscribeCOVPropertyMultiple admission on the wire (Clause 13.16.2; #1058,
//! #1059). The server processes the references in request order. The first
//! that fails, whether validation or a subscription cap refuses it, is named
//! in the error and ends the request: the references before it stay
//! subscribed and get their initial report, and those after it are never
//! processed. A failure of the request itself keeps and reports nothing.
use super::cov_wire_test_support::*;
use super::*;
use crate::cov::{CovRecipient, MultipleContextKey};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::error::ErrorDetail;

const OOS: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
const EVENT_STATE: PropertyIdentifier = PropertyIdentifier::EVENT_STATE;

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

fn with_av2(db: &mut ObjectDatabase) {
    db.add(Box::new(AnalogValueObject::new(2, "AV-2", 62).unwrap()))
        .unwrap();
}

/// `(object, [property])` specifications, none timestamped.
fn untimed(
    specs: &[(ObjectIdentifier, &[PropertyIdentifier])],
) -> Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, bool)>)> {
    specs
        .iter()
        .map(|(object, properties)| {
            (
                *object,
                properties
                    .iter()
                    .map(|property| (*property, false))
                    .collect(),
            )
        })
        .collect()
}

/// SubscribeCOVPropertyMultiple from `process` whose delay equals its
/// lifetime (300 s), which the checked request encoder refuses to produce,
/// over valid Present_Value and Out_Of_Service references of AV-1.
fn delay_not_below_lifetime(process: u32) -> BytesMut {
    use bacnet_encoding::{primitives, tags};
    let mut request = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut request, 0, u64::from(process));
    primitives::encode_ctx_boolean(&mut request, 1, false);
    primitives::encode_ctx_unsigned(&mut request, 2, 300);
    primitives::encode_ctx_unsigned(&mut request, 3, 300);
    tags::encode_opening_tag(&mut request, 4);
    primitives::encode_ctx_object_id(&mut request, 0, &av(1));
    tags::encode_opening_tag(&mut request, 1);
    for property in [PV, OOS] {
        tags::encode_opening_tag(&mut request, 0);
        primitives::encode_ctx_unsigned(&mut request, 0, u64::from(property.to_raw()));
        tags::encode_closing_tag(&mut request, 0);
        primitives::encode_ctx_boolean(&mut request, 2, false);
    }
    tags::encode_closing_tag(&mut request, 1);
    tags::encode_closing_tag(&mut request, 4);
    request
}

/// Wait for the server's answer to the last request: its SimpleACK or Error.
async fn answer(h: &Harness) -> Apdu {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames
                    .iter()
                    .position(|apdu| matches!(apdu, Apdu::SimpleAck(_) | Apdu::Error(_)));
                at.map(|at| frames.remove(at))
            };
            match next {
                Some(apdu) => return apdu,
                None => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    })
    .await
    .expect("an answer to the request")
}

/// Class, code and detail of an Error answer; no detail is the general choice.
fn refusal(apdu: Apdu) -> (ErrorClass, ErrorCode, Option<ErrorDetail>) {
    let Apdu::Error(error) = apdu else {
        panic!("expected an Error, got {apdu:?}");
    };
    let detail = bacnet_services::structured_error::detail(&error);
    (error.error_class, error.error_code, detail)
}

/// The first-failed-subscription detail naming `property` of `object`.
fn names(object: ObjectIdentifier, property: PropertyIdentifier) -> Option<ErrorDetail> {
    Some(ErrorDetail::FirstFailedSubscription(
        BACnetObjectPropertyReference::new(object, property.to_raw()),
    ))
}

/// The `(object, property)` pairs a notification reports, sorted and once each.
fn reported(notification: &COVNotificationMultipleRequest) -> Vec<(u32, PropertyIdentifier)> {
    let mut pairs: Vec<_> = notification
        .list_of_cov_notifications
        .iter()
        .flat_map(|item| {
            item.list_of_values.iter().map(|value| {
                (
                    item.monitored_object_identifier.instance_number(),
                    value.property_identifier,
                )
            })
        })
        .collect();
    pairs.sort_by_key(|(object, property)| (*object, property.to_raw()));
    pairs.dedup();
    pairs
}

fn context(process: u32) -> MultipleContextKey {
    MultipleContextKey {
        recipient: CovRecipient::from_endpoint(&PEER, None),
        process_id: process,
        confirmed: false,
    }
}

/// The `(object, property)` references of `process`'s unconfirmed context,
/// sorted.
async fn kept(h: &Harness, process: u32) -> Vec<(u32, PropertyIdentifier)> {
    let table = h.server.cov_table.read().await;
    let mut pairs: Vec<_> = table
        .multiple_context_references(&context(process))
        .map(|sub| {
            (
                sub.monitored_object_identifier.instance_number(),
                sub.monitored_property.unwrap(),
            )
        })
        .collect();
    pairs.sort_by_key(|(object, property)| (*object, property.to_raw()));
    pairs
}

/// The maximum notification delay each reference of `process`'s context holds.
async fn delays(h: &Harness, process: u32) -> Vec<Option<u32>> {
    let table = h.server.cov_table.read().await;
    table
        .multiple_context_references(&context(process))
        .map(|sub| sub.max_notification_delay())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_failure_at_the_second_or_third_reference_keeps_and_reports_the_ones_before() {
    let mut h = Harness::start_with(ServerConfig::default(), with_av2).await;
    let bogus = PropertyIdentifier::from_raw(4_000);

    // The second reference names a property AV-1 lacks.
    h.subscribe_process(61, false, untimed(&[(av(1), &[PV, bogus])]), Some(10))
        .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
            names(av(1), bogus)
        )
    );
    let initial = h.notification().await;
    assert_eq!(initial.subscriber_process_identifier, 61);
    assert_eq!(reported(&initial), [(1, PV), (1, SF)]);
    h.no_notification().await;
    assert_eq!(kept(&h, 61).await, [(1, PV)]);

    // The third names a missing object (AV-99); the valid AV-2
    // reference after it is never processed.
    h.subscribe_process(
        62,
        false,
        untimed(&[(av(1), &[PV, OOS]), (av(99), &[PV]), (av(2), &[PV])]),
        Some(10),
    )
    .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
            names(av(99), PV)
        )
    );
    let initial = h.notification().await;
    assert_eq!(initial.subscriber_process_identifier, 62);
    assert_eq!(reported(&initial), [(1, OOS), (1, PV), (1, SF)]);
    h.no_notification().await;
    assert_eq!(kept(&h, 62).await, [(1, OOS), (1, PV)]);
    assert!(h
        .server
        .cov_table
        .write()
        .await
        .subscriptions_for(&av(2))
        .is_empty());
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_reference_past_the_subscription_cap_is_named_and_the_ones_before_it_kept() {
    let config = ServerConfig {
        cov_policy: CovPolicy {
            max_subscriptions_per_peer: 1024,
            ..CovPolicy::default()
        },
        ..ServerConfig::default()
    };
    let mut h = Harness::start(config).await;
    // Another subscriber holds all but two of the table's 1,024 slots.
    {
        let mut table = h.server.cov_table.write().await;
        for process in 0..1022 {
            table
                .subscribe(crate::cov::CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 9, 0xBA, 0xC0]),
                    subscriber_network: None,
                    subscriber_process_identifier: process,
                    monitored_object_identifier: av(1),
                    issue_confirmed_notifications: false,
                    expires_at: Some(runtime_clock::now() + Duration::from_secs(3600)),
                    last_notified_observation: None,
                    monitored_property: None,
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: crate::cov::CovNotificationKind::Single,
                    timestamped: false,
                })
                .unwrap();
        }
    }
    let rejected = |h: &Harness| h.server.cov_counters().subscriptions_rejected_capacity;

    // Present_Value and Out_Of_Service take the last two slots; Event_State
    // is the reference that does not fit.
    h.subscribe_process(
        63,
        false,
        untimed(&[(av(1), &[PV, OOS, EVENT_STATE])]),
        Some(10),
    )
    .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            names(av(1), EVENT_STATE)
        )
    );
    assert_eq!(
        reported(&h.notification().await),
        [(1, OOS), (1, PV), (1, SF)]
    );
    h.no_notification().await;
    assert_eq!(kept(&h, 63).await, [(1, OOS), (1, PV)]);
    assert_eq!(h.server.cov_table.read().await.len(), 1024);
    assert_eq!(rejected(&h), 1);

    // At the cap a renewal still fits, as it adds no subscription: it is
    // processed and reported, and the new reference after it is named.
    h.subscribe_process(63, false, untimed(&[(av(1), &[PV, EVENT_STATE])]), Some(20))
        .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            names(av(1), EVENT_STATE)
        )
    );
    assert_eq!(reported(&h.notification().await), [(1, PV), (1, SF)]);
    h.no_notification().await;
    assert_eq!(kept(&h, 63).await, [(1, OOS), (1, PV)]);
    // The renewal refreshed the whole context, Out_Of_Service included.
    assert_eq!(delays(&h, 63).await, [Some(20), Some(20)]);
    assert_eq!(rejected(&h), 2);

    // A new first reference is refused before anything is processed: the
    // renewal after it does not happen and nothing is reported.
    h.subscribe_process(63, false, untimed(&[(av(1), &[EVENT_STATE, PV])]), Some(30))
        .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            names(av(1), EVENT_STATE)
        )
    );
    h.no_notification().await;
    assert_eq!(delays(&h, 63).await, [Some(20), Some(20)]);
    assert_eq!(rejected(&h), 3);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_refusal_of_the_whole_request_keeps_and_reports_nothing() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_process(64, false, untimed(&[(av(1), &[PV])]), Some(10))
        .await;
    assert!(matches!(answer(&h).await, Apdu::SimpleAck(_)));
    h.notification().await;
    let before = h.server.cov_table.read().await.len();

    // A delay not below the lifetime refuses the request before any
    // reference; the general choice carries no reference.
    h.request(
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
        delay_not_below_lifetime(64),
    )
    .await;
    assert_eq!(
        refusal(answer(&h).await),
        (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE, None)
    );
    h.no_notification().await;
    assert_eq!(kept(&h, 64).await, [(1, PV)]);
    assert_eq!(h.server.cov_table.read().await.len(), before);
    assert_eq!(delays(&h, 64).await, [Some(10)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn without_a_clock_the_timestamped_reference_is_refused_and_the_earlier_ones_stay() {
    let mut h = Harness::start_with(ServerConfig::default(), with_av2).await;
    h.server.database().write().await.set_clock_reader(None);
    // Timestamped is an option of each reference, so a missing clock refuses
    // only the reference that asks for it (#1102). The untimestamped ones
    // before it stay and get their initial report; the reference after it is
    // not processed.
    h.subscribe_process(
        66,
        false,
        vec![
            (av(1), vec![(PV, false), (OOS, false), (EVENT_STATE, true)]),
            (av(2), vec![(PV, false)]),
        ],
        Some(20),
    )
    .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::SERVICES,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            names(av(1), EVENT_STATE)
        )
    );
    assert_eq!(
        reported(&h.notification().await),
        [(1, OOS), (1, PV), (1, SF)]
    );
    h.no_notification().await;
    assert_eq!(kept(&h, 66).await, [(1, OOS), (1, PV)]);
    assert_eq!(delays(&h, 66).await, [Some(20), Some(20)]);

    // As the first reference it is refused alone, and nothing changes.
    h.subscribe_process(66, false, vec![(av(2), vec![(PV, true)])], Some(30))
        .await;
    assert_eq!(
        refusal(answer(&h).await),
        (
            ErrorClass::SERVICES,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            names(av(2), PV)
        )
    );
    h.no_notification().await;
    assert_eq!(kept(&h, 66).await, [(1, OOS), (1, PV)]);
    assert_eq!(delays(&h, 66).await, [Some(20), Some(20)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_cancellation_removes_every_listed_reference_whatever_precedes_it() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_process(65, false, untimed(&[(av(1), &[PV, OOS])]), Some(10))
        .await;
    assert!(matches!(answer(&h).await, Apdu::SimpleAck(_)));
    h.notification().await;
    // A cancellation validates nothing, so a reference to a missing object
    // does not stop the ones after it.
    h.subscribe_process(65, false, untimed(&[(av(99), &[PV]), (av(1), &[PV])]), None)
        .await;
    assert!(matches!(answer(&h).await, Apdu::SimpleAck(_)));
    h.no_notification().await;
    assert_eq!(kept(&h, 65).await, [(1, OOS)]);
    h.server.stop().await.unwrap();
}

/// `notification` carries Status_Flags alone: the Present_Value of its
/// change fit no notification and was dropped (#1090).
fn flags_only(notification: &COVNotificationMultipleRequest) {
    let properties: Vec<_> = notification
        .list_of_cov_notifications
        .iter()
        .flat_map(|item| &item.list_of_values)
        .map(|value| value.property_identifier)
        .collect();
    assert_eq!(properties, [SF]);
}

async fn write_text(h: &Harness, object: ObjectIdentifier, text: String) {
    h.server
        .write_local(
            &object,
            PV,
            None,
            PropertyValue::CharacterString(text),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_partly_admitted_request_warns_afresh_about_dropped_changes() {
    // As in the #1039 and #1090 tests: CSV-1's long Present_Value fits no
    // 206-octet notification even alone, so each timestamped change drops it,
    // and only the change's Status_Flags go out.
    use bacnet_objects::value_types::CharacterStringValueObject;
    let csv = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap();
    let warnings = crate::cov::timed::DropWarningCount::default();
    let _guard = warnings.install();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
    })
    .await;
    h.request_max_apdu = 206;
    write_text(&h, csv, "x".repeat(200)).await;
    let seen = |h: &Harness| {
        (
            h.server.cov_counters().timed_changes_dropped,
            warnings.get(),
        )
    };
    h.subscribe_specs(false, vec![(csv, vec![(PV, true)])])
        .await;
    assert!(matches!(answer(&h).await, Apdu::SimpleAck(_)));
    flags_only(&h.notification().await);
    h.no_notification().await;
    assert_eq!(seen(&h), (1, 1));

    // Re-admitting the reference ahead of a refused one counts as an
    // admission: its initial change drops the value and warns afresh.
    h.subscribe_specs(
        false,
        vec![(csv, vec![(PV, true)]), (av(99), vec![(PV, false)])],
    )
    .await;
    assert_eq!(refusal(answer(&h).await).1, ErrorCode::UNKNOWN_OBJECT);
    flags_only(&h.notification().await);
    h.no_notification().await;
    assert_eq!(seen(&h), (2, 2));

    // A request refused at its first reference admits nothing, so the next
    // dropped value does not warn again.
    h.subscribe_specs(
        false,
        vec![(av(99), vec![(PV, false)]), (csv, vec![(PV, true)])],
    )
    .await;
    assert_eq!(refusal(answer(&h).await).1, ErrorCode::UNKNOWN_OBJECT);
    h.set_clock(1);
    write_text(&h, csv, "y".repeat(200)).await;
    flags_only(&h.notification().await);
    h.no_notification().await;
    assert_eq!(seen(&h), (3, 2));
    h.server.stop().await.unwrap();
}
