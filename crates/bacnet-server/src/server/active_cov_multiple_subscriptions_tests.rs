//! Wire evidence for the live Device `Active_COV_Multiple_Subscriptions`
//! projection (Clause 12.11, Table 12-13 footnote 18; #814). Every read goes
//! through the running server's ReadProperty/ReadPropertyMultiple dispatch
//! unless it names `read_local`.
use super::*;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_types::primitives::{Date, Time};

const FLAGS: PropertyIdentifier = PropertyIdentifier::STATUS_FLAGS;
const ARRAY: PropertyIdentifier = PropertyIdentifier::PRIORITY_ARRAY;

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// One request: process, form, `(lifetime, delay)` or `None` to cancel, and
/// `(object, references)` specifications.
fn subscribe(
    process: u32,
    confirmed: bool,
    terms: Option<(u32, u32)>,
    specs: Vec<(ObjectIdentifier, Vec<MultipleReference>)>,
) -> (ConfirmedServiceChoice, BytesMut) {
    subscribe_cov_property_multiple(process, confirmed, terms, specs)
}

#[tokio::test]
async fn active_cov_multiple_wire_is_listed_readable_and_live_after_acceptance() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let listed = property_list(
        &wire
            .read(device(), PropertyIdentifier::PROPERTY_LIST, None)
            .await
            .unwrap(),
    );
    let before = wire.read(device(), MULTIPLE, None).await;
    let accepted = wire
        .send(
            &direct(),
            subscribe(71, false, Some((300, 10)), vec![(av(1), vec![plain(PV)])]),
        )
        .await;
    let after = wire.read(device(), MULTIPLE, None).await;
    let in_list = listed.contains(&MULTIPLE.to_raw());
    // On the pre-#814 baseline all three observations failed: the property
    // was absent from Property_List and ReadProperty returned UNKNOWN_PROPERTY
    // both before and after an accepted SubscribeCOVPropertyMultiple.
    assert!(
        in_list && before == Ok(Vec::new()) && after.as_ref().is_ok_and(|v| !v.is_empty()),
        "Property_List has 481: {in_list}; RP before: {before:?}; \
         SubscribeCOVPropertyMultiple: {accepted:?}; RP after acceptance: {after:?}"
    );
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_multiple_wire_groups_references_by_recipient_and_form() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    wire.server
        .database()
        .write()
        .await
        .set_clock_reader(Some(std::sync::Arc::new(FixedClock)));
    let unconfirmed = vec![
        (
            av(1),
            vec![(PV, None, Some(1.5), true), (ARRAY, Some(8), None, false)],
        ),
        (av(2), vec![plain(PV)]),
    ];
    simple_ack(
        wire.send(
            &direct(),
            subscribe(71, false, Some((300, 10)), unconfirmed),
        )
        .await,
    );
    // Same recipient and process, other form: an independent context.
    let confirmed = vec![(av(1), vec![plain(FLAGS)])];
    simple_ack(
        wire.send(&direct(), subscribe(71, true, Some((600, 20)), confirmed))
            .await,
    );
    let routed_refs = vec![(av(1), vec![plain(PV)])];
    simple_ack(
        wire.send(
            &routed(),
            subscribe(72, false, Some((900, 30)), routed_refs),
        )
        .await,
    );

    let listed = wire.multiple().await;
    let expected = vec![
        // Direct recipients precede routed ones; the unconfirmed form first.
        context(
            &direct(),
            71,
            false,
            10,
            vec![
                spec(
                    av(1),
                    vec![
                        // An explicit override is the numeric increment in use.
                        reference(PV, None, Some(1.5), true),
                        // A non-numeric selected element carries none.
                        reference(ARRAY, Some(8), None, false),
                    ],
                ),
                // Numeric Present_Value reports the object's COV_Increment.
                spec(av(2), vec![reference(PV, None, Some(0.0), false)]),
            ],
        ),
        context(
            &direct(),
            71,
            true,
            20,
            vec![spec(av(1), vec![reference(FLAGS, None, None, false)])],
        ),
        // A routed recipient is its remote NPDU source, never the router MAC.
        context(
            &routed(),
            72,
            false,
            30,
            vec![spec(av(1), vec![reference(PV, None, Some(0.0), false)])],
        ),
    ];
    assert_eq!(
        untimed(&listed, &[(299, 300), (599, 600), (899, 900)]),
        expected
    );
    // Multiple references never enter Active_COV_Subscriptions.
    assert_eq!(wire.active().await, Vec::new());
    // The increment in use follows the object; explicit overrides do not.
    wire.server
        .write_local(
            &av(2),
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(2.5),
            None,
        )
        .await
        .unwrap();
    let listed = wire.multiple().await;
    let first = &listed[0].list_of_cov_subscription_specifications;
    assert_eq!(first[0].list_of_cov_references[0].cov_increment, Some(1.5));
    assert_eq!(first[1].list_of_cov_references[0].cov_increment, Some(2.5));
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_multiple_wire_renewal_cancellation_expiry_and_cleanup_are_exact() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let both = vec![(av(1), vec![plain(PV), plain(FLAGS)])];
    simple_ack(
        wire.send(&direct(), subscribe(81, false, Some((300, 10)), both))
            .await,
    );
    let confirmed = vec![(av(1), vec![plain(PV)])];
    simple_ack(
        wire.send(&direct(), subscribe(81, true, Some((300, 11)), confirmed))
            .await,
    );
    let pv = |object| spec(object, vec![reference(PV, None, Some(0.0), false)]);
    let flags = reference(FLAGS, None, None, false);
    let pv_ref = reference(PV, None, Some(0.0), false);
    let confirmed_context = context(&direct(), 81, true, 11, vec![pv(av(1))]);

    // An empty-spec renewal refreshes the unconfirmed context's lifetime and
    // delay only; the confirmed form of the same recipient is untouched.
    simple_ack(
        wire.send(&direct(), subscribe(81, false, Some((600, 40)), vec![]))
            .await,
    );
    let refreshed = context(
        &direct(),
        81,
        false,
        40,
        vec![spec(av(1), vec![pv_ref.clone(), flags.clone()])],
    );
    assert_eq!(
        untimed(&wire.multiple().await, &[(599, 600), (299, 300)]),
        vec![refreshed, confirmed_context.clone()]
    );
    // A re-subscription adds a reference and refreshes the whole context.
    let added = vec![(av(2), vec![plain(PV)])];
    simple_ack(
        wire.send(&direct(), subscribe(81, false, Some((700, 50)), added))
            .await,
    );
    let modified = |specs| context(&direct(), 81, false, 50, specs);
    assert_eq!(
        untimed(&wire.multiple().await, &[(699, 700), (299, 300)]),
        vec![
            modified(vec![
                spec(av(1), vec![pv_ref.clone(), flags.clone()]),
                pv(av(2))
            ]),
            confirmed_context.clone()
        ]
    );
    // Cancelling one reference keeps the context and its terms.
    let cancel_flags = vec![(av(1), vec![plain(FLAGS)])];
    simple_ack(
        wire.send(&direct(), subscribe(81, false, None, cancel_flags))
            .await,
    );
    assert_eq!(
        untimed(&wire.multiple().await, &[(699, 700), (299, 300)]),
        vec![
            modified(vec![pv(av(1)), pv(av(2))]),
            confirmed_context.clone()
        ]
    );
    // Cancelling the entire context removes it; an absent context cancels as success.
    for _ in 0..2 {
        simple_ack(
            wire.send(&direct(), subscribe(81, false, None, vec![]))
                .await,
        );
        assert_eq!(
            untimed(&wire.multiple().await, &[(299, 300)]),
            vec![confirmed_context.clone()]
        );
    }

    // A sub-second context lifetime is table-only; the wire minimum is one second.
    let short = crate::cov::MultipleContextKey {
        endpoint: crate::cov::SubscriberEndpoint::new(&routed().mac, routed().network.as_ref()),
        process_id: 83,
        confirmed: false,
    };
    let expires_at = Instant::now() + Duration::from_millis(300);
    let proposal = CovSubscription {
        subscriber_mac: MacAddr::from_slice(&routed().mac),
        subscriber_network: routed().network,
        subscriber_process_identifier: 83,
        monitored_object_identifier: av(1),
        issue_confirmed_notifications: false,
        expires_at: Some(expires_at),
        last_notified_observation: None,
        monitored_property: Some(PV),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Multiple,
        timestamped: false,
    };
    wire.server
        .cov_table
        .write()
        .await
        .subscribe_multiple(&short, expires_at, 0, vec![proposal])
        .unwrap();
    let listed = wire.multiple().await;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].time_remaining, 1, "positive fractions round up");
    // Expired at the sampled instant: omitted before the periodic purge runs.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        untimed(&wire.multiple().await, &[(299, 300)]),
        vec![confirmed_context.clone()]
    );
    assert_eq!(wire.server.cov_table.read().await.len(), 2, "not purged");

    // DeleteObject terminates references on the deleted object; a context
    // left without references disappears entirely.
    let only_av2 = vec![(av(2), vec![plain(PV)])];
    simple_ack(
        wire.send(&routed(), subscribe(84, true, Some((300, 5)), only_av2))
            .await,
    );
    assert_eq!(wire.multiple().await.len(), 2);
    let mut delete = BytesMut::new();
    bacnet_services::object_mgmt::DeleteObjectRequest {
        object_identifier: av(2),
    }
    .encode(&mut delete);
    simple_ack(
        wire.send(&direct(), (ConfirmedServiceChoice::DELETE_OBJECT, delete))
            .await,
    );
    assert_eq!(
        untimed(&wire.multiple().await, &[(299, 300)]),
        vec![confirmed_context]
    );

    // Exact peer cleanup removes the matching recipient's contexts.
    let refs = vec![(av(1), vec![plain(PV)])];
    simple_ack(
        wire.send(&routed(), subscribe(85, false, Some((300, 5)), refs))
            .await,
    );
    assert_eq!(wire.multiple().await.len(), 2);
    let source = routed().network;
    assert_eq!(
        wire.server
            .remove_peer_subscriptions(&[0xFF], source.as_ref())
            .await,
        0,
        "another router carrying the same source keeps it"
    );
    assert_eq!(wire.multiple().await.len(), 2);
    assert_eq!(
        wire.server
            .remove_peer_subscriptions(&routed().mac, source.as_ref())
            .await,
        1
    );
    let listed = wire.multiple().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].recipient.recipient, address(&direct()));
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_multiple_wire_empty_spec_and_rejected_input_leave_the_list_unchanged() {
    let config = ServerConfig {
        cov_policy: CovPolicy {
            max_subscriptions_per_peer: 2,
            ..CovPolicy::default()
        },
        ..ServerConfig::default()
    };
    let mut wire = Wire::start(config).await;
    wire.server.database().write().await.set_clock_reader(None);
    // An accepted empty-spec finite request establishes no observable context.
    simple_ack(
        wire.send(&direct(), subscribe(91, false, Some((300, 10)), vec![]))
            .await,
    );
    assert_eq!(wire.multiple_bytes().await, Vec::<u8>::new());

    let refs = vec![(av(1), vec![plain(PV)])];
    simple_ack(
        wire.send(&direct(), subscribe(92, false, Some((300, 10)), refs))
            .await,
    );
    let baseline = vec![context(
        &direct(),
        92,
        false,
        10,
        vec![spec(av(1), vec![reference(PV, None, Some(0.0), false)])],
    )];
    assert_eq!(untimed(&wire.multiple().await, &[(299, 300)]), baseline);

    // Each rejected re-subscription carries a new lifetime and delay; none
    // changes the context, its references, lifetime or reported delay.
    let rejected = [
        (
            // Atomic: the valid Status_Flags reference is not added either.
            subscribe(
                92,
                false,
                Some((900, 99)),
                vec![(av(1), vec![plain(FLAGS)]), (av(99), vec![plain(PV)])],
            ),
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            subscribe(
                92,
                false,
                Some((900, 99)),
                vec![(av(1), vec![plain(MULTIPLE)])],
            ),
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            subscribe(
                92,
                false,
                Some((900, 99)),
                vec![(device(), vec![plain(PV)])],
            ),
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        ),
        (
            // A clockless server cannot report timestamps.
            subscribe(
                92,
                false,
                Some((900, 99)),
                vec![(av(1), vec![(FLAGS, None, None, true)])],
            ),
            ErrorClass::SERVICES,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        ),
        (
            // Two new references exceed the two-per-peer quota.
            subscribe(
                92,
                false,
                Some((900, 99)),
                vec![(av(1), vec![plain(FLAGS)]), (av(2), vec![plain(PV)])],
            ),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
        ),
        // Lifetime zero would be indefinite; Multiple contexts are finite.
        (
            out_of_range(92, 0, 0),
            ErrorClass::SERVICES,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            out_of_range(92, 900, 900),
            ErrorClass::SERVICES,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ];
    for (request, class, code) in rejected {
        error(wire.send(&direct(), request).await, class, code);
        assert_eq!(untimed(&wire.multiple().await, &[(299, 300)]), baseline);
    }
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_multiple_wire_rpm_rp_and_local_reads_agree_within_scope() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(&direct(), subscribe_cov(51, av(1), Some(true), None))
            .await,
    );
    let refs = vec![(av(1), vec![(PV, None, Some(0.25), false)])];
    simple_ack(
        wire.send(&routed(), subscribe(52, true, Some((300, 7)), refs))
            .await,
    );

    let ack = wire
        .rpm(vec![
            (
                device(),
                vec![
                    (MULTIPLE, None),
                    (MULTIPLE, None),
                    (PropertyIdentifier::ALL, None),
                    (PropertyIdentifier::OPTIONAL, None),
                ],
            ),
            (wildcard(), vec![(MULTIPLE, None)]),
            (av(1), vec![(PV, None)]),
        ])
        .await;
    let multiple = rows(&ack, MULTIPLE);
    assert_eq!(multiple.len(), 5, "explicit x2, ALL, OPTIONAL and wildcard");
    assert!(
        multiple.iter().all(|row| row == &multiple[0]),
        "one snapshot"
    );
    let expected = vec![context(
        &routed(),
        52,
        true,
        7,
        vec![spec(av(1), vec![reference(PV, None, Some(0.25), false)])],
    )];
    assert_eq!(
        untimed(&decode_contexts(&multiple[0]), &[(299, 300)]),
        expected
    );
    assert_eq!(untimed(&wire.multiple().await, &[(299, 300)]), expected);
    // Active_COV_Subscriptions rows in the same response list only the
    // ordinary entry.
    let ordinary = rows(&ack, ACTIVE);
    assert_eq!(ordinary.len(), 2, "ALL and OPTIONAL");
    assert_eq!(processes(&decode_subscriptions(&ordinary[0])), vec![51]);

    // Renewal/cancellation racing repeated references never splits one response.
    let toggler = {
        let tx = wire.tx.clone();
        tokio::spawn(async move {
            for invoke_id in 100..160u8 {
                let request = if invoke_id % 2 == 0 {
                    let refs = vec![(av(2), vec![plain(PV)])];
                    subscribe(99, false, Some((300, 1)), refs)
                } else {
                    subscribe(99, false, None, vec![])
                };
                simple_ack(exchange(&tx, &routed(), invoke_id, request).await);
            }
        })
    };
    for _ in 0..30 {
        let ack = wire
            .rpm(vec![
                (
                    device(),
                    vec![
                        (MULTIPLE, None),
                        (MULTIPLE, None),
                        (PropertyIdentifier::ALL, None),
                    ],
                ),
                (wildcard(), vec![(MULTIPLE, None)]),
            ])
            .await;
        let multiple = rows(&ack, MULTIPLE);
        assert_eq!(multiple.len(), 4);
        assert!(
            multiple.iter().all(|row| row == &multiple[0]),
            "one snapshot"
        );
    }
    toggler.await.unwrap();

    // Read-only, non-array list: errors precede the live value.
    let live = wire.multiple_bytes().await;
    let mut write = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: device(),
        property_identifier: MULTIPLE,
        property_array_index: None,
        property_value: vec![0x00],
        priority: None,
    }
    .encode(&mut write)
    .unwrap();
    let write = (ConfirmedServiceChoice::WRITE_PROPERTY, write);
    let response = wire.send(&direct(), write).await;
    error(
        response,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    let not_array = (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY);
    assert_eq!(wire.read(device(), MULTIPLE, Some(1)).await, Err(not_array));
    let ack = wire.rpm(vec![(device(), vec![(MULTIPLE, Some(0))])]).await;
    let row = &ack.list_of_read_access_results[0].list_of_results[0];
    assert_eq!(
        (row.error, row.property_array_index),
        (Some(not_array), None)
    );
    let unknown = ObjectIdentifier::new(ObjectType::DEVICE, 777).unwrap();
    let unknown_object = (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    assert_eq!(
        wire.read(unknown, MULTIPLE, None).await,
        Err(unknown_object)
    );

    // Only the selected local Device is live; another Device keeps its
    // standalone default list, as do raw object reads.
    let other = ObjectIdentifier::new(ObjectType::DEVICE, 900).unwrap();
    let config = DeviceConfig {
        instance: 900,
        name: "Other Device".into(),
        ..DeviceConfig::default()
    };
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(DeviceObject::new(config).unwrap()))
        .unwrap();
    let selected =
        handlers::resolve_device_wildcard(&*wire.server.database().read().await, &wildcard());
    let (live_device, standalone) = if selected == device() {
        (device(), other)
    } else {
        (other, device())
    };
    let empty = Vec::<u8>::new();
    assert_eq!(wire.read(live_device, MULTIPLE, None).await.unwrap(), live);
    assert_eq!(wire.read(standalone, MULTIPLE, None).await.unwrap(), empty);
    let local = PropertyValue::ApplicationData(live.clone());
    for oid in [live_device, wildcard()] {
        assert_eq!(
            wire.server.read_local(&oid, MULTIPLE, None).await.unwrap(),
            local
        );
    }
    let raw = wire
        .server
        .database()
        .read()
        .await
        .get(&live_device)
        .unwrap()
        .read_property(MULTIPLE, None);
    assert_eq!(raw.unwrap(), PropertyValue::ApplicationData(Vec::new()));

    // A stopped server services no context and reports none; the table is
    // not consulted as a stale fallback.
    wire.server.stop().await.unwrap();
    assert!(!wire.server.cov_table.read().await.is_empty());
    assert_eq!(
        wire.server
            .read_local(&live_device, MULTIPLE, None)
            .await
            .unwrap(),
        PropertyValue::ApplicationData(Vec::new())
    );
}
