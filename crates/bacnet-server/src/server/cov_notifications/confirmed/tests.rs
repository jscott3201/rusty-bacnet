//! Admission refusals and follow-up budgets of confirmed COV reports (#896),
//! driven straight through the sender and the follow-up fanout.
use super::*;
use crate::cov::{CovNotificationKind, CovSample, CovSubscription};
use crate::server::cov_notify_context::CovNotifyContext;
use crate::server::test_transport::{SendLog, SendMode, TestTransport, BIP_LOCAL_MAC};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use std::collections::HashSet;

struct Fixture {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<TestTransport>>,
    sent: SendLog,
    table: Arc<RwLock<CovSubscriptionTable>>,
    permits: Arc<Semaphore>,
    transactions: Arc<NotificationTransactions>,
    comm: Arc<CommState>,
    config: Arc<ServerConfig>,
}

fn av1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

/// A confirmed subscription to AV-1: whole-object, or one Multiple reference.
fn proposal(kind: CovNotificationKind, property: PropertyIdentifier) -> CovSubscription {
    proposal_on(av1(), kind, property)
}

fn proposal_on(
    object: ObjectIdentifier,
    kind: CovNotificationKind,
    property: PropertyIdentifier,
) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
        subscriber_network: None,
        subscriber_process_identifier: 7,
        monitored_object_identifier: object,
        issue_confirmed_notifications: true,
        expires_at: Some(runtime_clock::now() + Duration::from_secs(3600)),
        last_notified_observation: None,
        monitored_property: (kind == CovNotificationKind::Multiple).then_some(property),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}

impl Fixture {
    fn new(config: ServerConfig) -> Self {
        let mut db = crate::server::clock::clocked_test_database();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 896,
                name: "Confirmed COV".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        db.add(Box::new(AnalogValueObject::new(1, "AV-1", 62).unwrap()))
            .unwrap();
        let transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .broadcast(SendMode::Ignore)
            .build();
        let sent = transport.sent();
        Self {
            db: Arc::new(RwLock::new(db)),
            network: Arc::new(NetworkLayer::new(transport)),
            sent,
            table: Arc::new(RwLock::new(CovSubscriptionTable::new())),
            permits: Arc::new(Semaphore::new(255)),
            transactions: NotificationTransactions::new(),
            comm: Arc::new(CommState::default()),
            config: Arc::new(config),
        }
    }

    fn ctx(&self) -> CovNotifyContext<'_, TestTransport> {
        CovNotifyContext {
            db: &self.db,
            network: &self.network,
            cov_table: &self.table,
            cov_in_flight: &self.permits,
            notification_transactions: &self.transactions,
            comm_state: &self.comm,
            config: &self.config,
        }
    }

    async fn admit(&self, sub: CovSubscription) -> CovSubscriptionSnapshot {
        self.table.write().await.admit_for_test(sub, 0).unwrap()
    }

    /// Offer one confirmed report of `sub` to the sender.
    async fn send(&self, sub: &CovSubscriptionSnapshot, budget: &mut EventBudget) {
        let (counters, in_flight_tracker) = {
            let table = self.table.read().await;
            (
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
            )
        };
        let ctx = self.ctx();
        let observation =
            CovObservation::new(CovSample::new(&PropertyValue::Real(1.0)).unwrap(), None).unwrap();
        BACnetServer::<TestTransport>::send_confirmed_cov(
            &CovFanoutHandles {
                ctx: &ctx,
                in_flight_tracker: &in_flight_tracker,
                counters: &counters,
            },
            budget,
            ConfirmedReport {
                service: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                route: sub.clone(),
                completion: sub.prepare_completion().unwrap(),
                observations: vec![(sub.clone(), observation)],
                claim: None,
                deferred: Vec::new(),
            },
            |_| Ok(BytesMut::from(&[0u8; 8][..])),
        )
        .await;
    }

    /// The refused report left no trace: no send, lease, permit, budget or
    /// counter spent.
    async fn untouched(&self, budget: &EventBudget) {
        assert!(self.sent.is_empty());
        assert_eq!(self.transactions.active_count(), 0);
        assert_eq!(self.permits.available_permits(), 255);
        assert_eq!(
            budget.remaining_notifications(),
            self.config.cov_policy.max_notifications_per_event,
            "the budget is refunded"
        );
        let table = self.table.read().await;
        assert_eq!(table.counters().snapshot().notifications_sent, 0);
        assert_eq!(table.in_flight_tracker().active_peer_count(), 0);
    }

    async fn finish(&self) {
        self.transactions.close();
        while let Some(joined) = self.transactions.join_next().await {
            assert!(joined.is_ok() || joined.is_err_and(|error| error.is_cancelled()));
        }
    }
}

#[tokio::test]
async fn a_busy_coordinate_refuses_a_report_and_leaves_the_follow_up_to_it() {
    let f = Fixture::new(ServerConfig::default());
    let single = f
        .admit(proposal(
            CovNotificationKind::Single,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let a = f
        .admit(proposal(
            CovNotificationKind::Multiple,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let b = f
        .admit(proposal(
            CovNotificationKind::Multiple,
            PropertyIdentifier::STATUS_FLAGS,
        ))
        .await;
    // An outstanding report of the subscription, and of a sibling reference.
    let outstanding = {
        let mut table = f.table.write().await;
        [&single, &a].map(|sub| {
            table
                .begin_confirmed(sub.prepare_completion().unwrap(), [sub])
                .unwrap()
        })
    };
    for sub in [&single, &b] {
        let mut budget = EventBudget::new(&f.config.cov_policy);
        f.send(sub, &mut budget).await;
        f.untouched(&budget).await;
        assert!(
            f.table.read().await.revisits().queued().is_empty(),
            "{:?}: the outstanding report owns the follow-up",
            sub.notification_kind
        );
    }
    drop(outstanding);
    f.finish().await;
}

#[tokio::test]
async fn a_fenced_reference_refuses_a_report_and_is_evaluated_again() {
    let f = Fixture::new(ServerConfig::default());
    let single = f
        .admit(proposal(
            CovNotificationKind::Single,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let reference = f
        .admit(proposal(
            CovNotificationKind::Multiple,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    // Renewal and re-subscription replace both after they were captured.
    f.admit((*single).clone()).await;
    f.admit((*reference).clone()).await;
    for sub in [&single, &reference] {
        let mut budget = EventBudget::new(&f.config.cov_policy);
        f.send(sub, &mut budget).await;
        f.untouched(&budget).await;
    }
    assert_eq!(
        f.table.read().await.revisits().queued(),
        HashSet::from([single.key().clone(), reference.key().clone()])
    );
    f.finish().await;
}

#[tokio::test]
async fn follow_ups_budget_each_object_and_context_separately() {
    let mut config = ServerConfig::default();
    config.cov_policy.max_notifications_per_event = 1;
    let f = Fixture::new(config);
    let single = f
        .admit(proposal(
            CovNotificationKind::Single,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let reference = f
        .admit(proposal(
            CovNotificationKind::Multiple,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    // Each owes a first report; one shared budget of one would drop the second.
    BACnetServer::<TestTransport>::fire_cov_revisits(
        &f.ctx(),
        &[single.key().clone(), reference.key().clone()],
    )
    .await;
    tokio::time::timeout(Duration::from_secs(1), f.sent.wait_for_len(2))
        .await
        .expect("a report per object and per context");
    let services: HashSet<_> = f
        .sent
        .frames()
        .iter()
        .map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request) => request.service_choice,
            other => panic!("expected a confirmed notification: {other:?}"),
        })
        .collect();
    assert_eq!(
        services,
        HashSet::from([
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ])
    );
    let counters = f.table.read().await.counters().snapshot();
    assert_eq!(counters.notifications_throttled_fanout, 0);
    f.finish().await;
}

#[tokio::test(start_paused = true)]
async fn a_fence_follow_up_that_beats_the_initial_report_carries_it() {
    let f = Fixture::new(ServerConfig::default());
    let a = f
        .admit(proposal(
            CovNotificationKind::Multiple,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let b = f
        .admit(proposal(
            CovNotificationKind::Multiple,
            PropertyIdentifier::STATUS_FLAGS,
        ))
        .await;
    let outstanding = {
        let mut table = f.table.write().await;
        table
            .begin_confirmed(a.prepare_completion().unwrap(), [&a, &b])
            .unwrap()
    };
    // Re-subscribing to Present_Value alone while the context is busy fences
    // its report and queues the whole context.
    let relisted = f
        .table
        .write()
        .await
        .subscribe_multiple(
            a.key().multiple_context().unwrap(),
            &a.endpoint(),
            a.expires_at.unwrap(),
            0,
            None,
            vec![(*a).clone()],
        )
        .unwrap();
    let queued: Vec<_> = f
        .table
        .read()
        .await
        .revisits()
        .queued()
        .into_iter()
        .collect();
    assert_eq!(queued.len(), 2);
    // The follow-up wins the race; the handler's initial report then finds the
    // context busy and sends nothing.
    BACnetServer::<TestTransport>::fire_cov_revisits(&f.ctx(), &queued).await;
    BACnetServer::<TestTransport>::fire_initial_cov_notification_multiple(&f.ctx(), &relisted)
        .await;
    tokio::time::timeout(Duration::from_secs(1), f.sent.wait_for_len(1))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), f.sent.wait_for_len(2))
            .await
            .is_err(),
        "one report for the context"
    );
    let Apdu::ConfirmedRequest(request) = f.sent.frame(0).apdu() else {
        panic!("confirmed COVNotificationMultiple");
    };
    let report = bacnet_services::cov_multiple::COVNotificationMultipleRequest::decode(
        &request.service_request,
    )
    .unwrap();
    let properties: HashSet<_> = report.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .map(|value| value.property_identifier)
        .collect();
    assert!(
        properties.contains(&PropertyIdentifier::PRESENT_VALUE),
        "the relisted reference's first report rides with the follow-up"
    );
    drop(outstanding);
    f.finish().await;
}

/// An analog value whose first Present_Value read panics.
struct PanicsOnce {
    panicked: Arc<std::sync::atomic::AtomicBool>,
}

impl bacnet_objects::traits::BACnetObject for PanicsOnce {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
    }

    fn object_name(&self) -> &str {
        "PANICS-ONCE"
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Borrowed(&[PropertyIdentifier::PRESENT_VALUE])
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property != PropertyIdentifier::PRESENT_VALUE {
            return Err(Error::Protocol {
                class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            });
        }
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "injected Present_Value read panic"
        );
        Ok(PropertyValue::Real(1.0))
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
            code: bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }
}

#[tokio::test]
async fn a_panicking_follow_up_batch_leaves_the_task_running() {
    let f = Fixture::new(ServerConfig::default());
    let panicked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    f.db.write()
        .await
        .add(Box::new(PanicsOnce {
            panicked: Arc::clone(&panicked),
        }))
        .unwrap();
    let broken = f
        .admit(proposal_on(
            ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap(),
            CovNotificationKind::Single,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let healthy = f
        .admit(proposal(
            CovNotificationKind::Single,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let fanout = crate::server::cov_fanout::CovFanout::new(&f.ctx(), &Default::default());
    let task = tokio::spawn(fanout.run_revisits());
    let revisits = Arc::clone(f.table.read().await.revisits());
    revisits.request([broken.key().clone()]);
    tokio::time::timeout(Duration::from_secs(1), async {
        while !panicked.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    revisits.request([healthy.key().clone()]);
    tokio::time::timeout(Duration::from_secs(1), f.sent.wait_for_len(1))
        .await
        .expect("the follow-up task survived the panicking batch");
    assert!(!task.is_finished());
    task.abort();
    f.finish().await;
}

/// DCC taking effect after a report is admitted, and counted, but before its
/// first attempt withdraws it unsent; its counts are taken back (#1327).
#[tokio::test(start_paused = true)]
async fn a_report_dcc_withdraws_before_its_first_attempt_is_not_counted() {
    let f = Fixture::new(ServerConfig::default());
    let sub = f
        .admit(proposal(
            CovNotificationKind::Single,
            PropertyIdentifier::PRESENT_VALUE,
        ))
        .await;
    let mut budget = EventBudget::new(&f.config.cov_policy);
    f.send(&sub, &mut budget).await;
    let admitted = f.table.read().await.counters().snapshot();
    assert_eq!(
        (
            admitted.notifications_sent,
            admitted.notifications_confirmed
        ),
        (1, 1)
    );
    f.comm.set_for_test(DccState::DisableInitiation); // DISABLE_INITIATION
    let joined = f.transactions.join_next().await;
    assert!(matches!(joined, Some(Ok(()))), "{joined:?}");
    assert!(f.sent.is_empty());
    assert_eq!(f.transactions.active_count(), 0);
    let counters = f.table.read().await.counters().snapshot();
    assert_eq!(
        (
            counters.notifications_sent,
            counters.notifications_confirmed,
            counters.notification_bytes_sent
        ),
        (0, 0, 0)
    );
    assert!(f.table.read().await.confirmed_idle(&sub), "no hold-off");
    f.finish().await;
}
