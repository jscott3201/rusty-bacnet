use super::*;
use crate::server::test_transport::{SendMode, SentFrame, TestTransport, BIP_LOCAL_MAC};
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::analog::AnalogValueObject;
use bytes::Bytes;
use std::sync::Mutex as StdMutex;
use tokio::sync::Notify;

type SendGates = Arc<StdMutex<std::collections::VecDeque<Arc<completion_order::CallGate>>>>;

/// The fixture's send path, installed as the shared test transport's send hook.
#[derive(Clone)]
struct HeldSends {
    gates: SendGates,
    sent: Arc<StdMutex<Vec<Bytes>>>,
    routes: Arc<StdMutex<Vec<MacAddr>>>,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
    hold: bool,
    fail: Arc<std::sync::atomic::AtomicBool>,
}
impl HeldSends {
    async fn send(self, frame: SentFrame) -> Result<(), Error> {
        let gate = self.gates.lock().unwrap().pop_front();
        if let Some(gate) = gate.as_ref().filter(|gate| !gate.record_before) {
            gate.wait().await?;
        }
        self.sent.lock().unwrap().push(frame.npdu);
        self.routes.lock().unwrap().push(frame.mac);
        self.entered.notify_one();
        if let Some(gate) = gate.as_ref().filter(|gate| gate.record_before) {
            gate.wait().await?;
        }
        if self.hold {
            self.release.acquire().await.unwrap().forget();
        }
        if self.fail.load(Ordering::Relaxed) {
            return Err(Error::Encoding("injected unconfirmed send failure".into()));
        }
        Ok(())
    }
    fn transport(self) -> TestTransport {
        TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .broadcast(SendMode::Ignore)
            .on_send(move |frame| self.clone().send(frame))
            .build()
    }
}

fn object() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 3).unwrap()
}
fn proposal(
    kind: CovNotificationKind,
    confirmed: bool,
    property: PropertyIdentifier,
) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xba, 0xd0]),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: object(),
        issue_confirmed_notifications: confirmed,
        // A Multiple context always has a finite lifetime.
        expires_at: (kind == CovNotificationKind::Multiple)
            .then(|| Instant::now() + Duration::from_secs(3600)),
        last_notified_observation: Some(
            crate::cov::CovObservation::new(
                crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(1.0))
                    .unwrap(),
                None,
            )
            .unwrap(),
        ),
        monitored_property: Some(property),
        monitored_property_array_index: None,
        cov_increment: Some(0.1),
        notification_kind: kind,
        timestamped: false,
    }
}

struct Fixture {
    gates: SendGates,
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<TestTransport>>,
    table: Arc<RwLock<CovSubscriptionTable>>,
    permits: Arc<Semaphore>,
    transactions: Arc<NotificationTransactions>,
    comm: Arc<CommState>,
    config: Arc<ServerConfig>,
    fail: Arc<std::sync::atomic::AtomicBool>,
    sent: Arc<StdMutex<Vec<Bytes>>>,
    routes: Arc<StdMutex<Vec<MacAddr>>>,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
}
impl Fixture {
    fn new(hold: bool) -> Self {
        let mut db = clocked_test_database();
        let mut av = AnalogValueObject::new(3, "value", 95).unwrap();
        av.set_relinquish_default(10.0).unwrap();
        db.add(Box::new(av)).unwrap();
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let routes = Arc::new(StdMutex::new(Vec::new()));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Semaphore::new(0));
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gates = SendGates::default();
        Self {
            gates: gates.clone(),
            db: Arc::new(RwLock::new(db)),
            network: Arc::new(NetworkLayer::new(
                HeldSends {
                    gates,
                    sent: sent.clone(),
                    routes: routes.clone(),
                    entered: entered.clone(),
                    release: release.clone(),
                    hold,
                    fail: fail.clone(),
                }
                .transport(),
            )),
            table: Arc::new(RwLock::new(CovSubscriptionTable::new())),
            permits: Arc::new(Semaphore::new(8)),
            transactions: NotificationTransactions::new(),
            comm: Arc::new(CommState::default()),
            config: Arc::default(),
            fail,
            sent,
            routes,
            entered,
            release,
        }
    }
    async fn fire(&self, initial: bool, snapshots: &[CovSubscriptionSnapshot]) {
        if !initial {
            BACnetServer::<TestTransport>::fire_cov_notifications(
                &crate::server::cov_notify_context::CovNotifyContext {
                    db: &self.db,
                    network: &self.network,
                    cov_table: &self.table,
                    cov_in_flight: &self.permits,
                    notification_transactions: &self.transactions,
                    comm_state: &self.comm,
                    config: &self.config,
                },
                &object(),
            )
            .await;
        } else if snapshots[0].notification_kind == CovNotificationKind::Single {
            BACnetServer::<TestTransport>::fire_initial_cov_notification(
                &crate::server::cov_notify_context::CovNotifyContext {
                    db: &self.db,
                    network: &self.network,
                    cov_table: &self.table,
                    cov_in_flight: &self.permits,
                    notification_transactions: &self.transactions,
                    comm_state: &self.comm,
                    config: &self.config,
                },
                &snapshots[0],
            )
            .await;
        } else {
            BACnetServer::<TestTransport>::fire_initial_cov_notification_multiple(
                &crate::server::cov_notify_context::CovNotifyContext {
                    db: &self.db,
                    network: &self.network,
                    cov_table: &self.table,
                    cov_in_flight: &self.permits,
                    notification_transactions: &self.transactions,
                    comm_state: &self.comm,
                    config: &self.config,
                },
                snapshots,
            )
            .await;
        }
    }
    async fn finish(&self, confirmed: bool) {
        if confirmed {
            tokio::time::timeout(Duration::from_secs(2), self.entered.notified())
                .await
                .unwrap();
            let frame = self.sent.lock().unwrap()[0].clone();
            let Apdu::ConfirmedRequest(request) =
                decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
            else {
                panic!("confirmed COV")
            };
            assert!(self.transactions.admit_terminal(
                &[127, 0, 0, 1, 0xba, 0xd0],
                None,
                None,
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice
                })
            ));
            // The worker completes baselines on the Ack (#896); let it.
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(2), self.transactions.join_next())
                    .await
                    .unwrap(),
                Some(Ok(()))
            ));
        }
        self.transactions.close();
        while self.transactions.join_next().await.is_some() {}
        assert_eq!(self.permits.available_permits(), 8);
    }
}

#[derive(Clone, Copy, Debug)]
enum Change {
    Renew,
    Recreate,
    Remove,
}

async fn stale_completion(
    initial: bool,
    kind: CovNotificationKind,
    confirmed: bool,
    change: Change,
    property: PropertyIdentifier,
) {
    let fixture = Fixture::new(!confirmed);
    let original = proposal(kind, confirmed, property);
    let mut snapshots = vec![fixture
        .table
        .write()
        .await
        .admit_for_test(original.clone(), 0)
        .unwrap()];
    if kind == CovNotificationKind::Multiple {
        snapshots.push(
            fixture
                .table
                .write()
                .await
                .admit_for_test(
                    proposal(kind, confirmed, PropertyIdentifier::COV_INCREMENT),
                    0,
                )
                .unwrap(),
        );
    }
    // Confirmed baseline is assigned on the Ack (#896), so hold the actual
    // object-read boundary after the snapshot was captured, not a mock setter.
    // Unconfirmed baseline is assigned after success: hold the actual send instead.
    let db_guard = if confirmed {
        Some(fixture.db.write().await)
    } else {
        None
    };
    let mut work = Box::pin(fixture.fire(initial, &snapshots));
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    if !confirmed {
        assert_eq!(fixture.sent.lock().unwrap().len(), 1);
    }
    {
        let mut table = fixture.table.write().await;
        if matches!(change, Change::Recreate | Change::Remove) {
            assert!(table.unsubscribe(snapshots[0].key()));
        }
        if !matches!(change, Change::Remove) {
            let mut renewed = original;
            renewed.last_notified_observation = Some(
                crate::cov::CovObservation::new(
                    crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(
                        99.0,
                    ))
                    .unwrap(),
                    None,
                )
                .unwrap(),
            );
            table.admit_for_test(renewed, 0).unwrap();
        }
    }
    drop(db_guard);
    fixture.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), work)
        .await
        .unwrap();
    let admitted = confirmed && kind == CovNotificationKind::Multiple;
    if confirmed && !admitted {
        assert!(
            fixture.sent.lock().unwrap().is_empty(),
            "late stale Single ownership must suppress admission"
        );
        assert_eq!(fixture.transactions.active_count(), 0);
    }
    fixture.finish(admitted).await;
    {
        let table = fixture.table.read().await;
        if matches!(change, Change::Remove) {
            assert!(table.get_subscription(snapshots[0].key()).is_none());
        } else {
            assert_eq!(table.get_subscription(snapshots[0].key()).unwrap().last_notified_observation,Some(crate::cov::CovObservation::new(crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(99.0)).unwrap(), None).unwrap()),"stale completion initial={initial} kind={kind:?} confirmed={confirmed} change={change:?}");
        }
        if kind == CovNotificationKind::Multiple {
            assert_eq!(
                table
                    .get_subscription(snapshots[1].key())
                    .unwrap()
                    .last_notified_observation,
                Some(
                    crate::cov::CovObservation::new(
                        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(
                            0.0
                        ))
                        .unwrap(),
                        Some(&PropertyValue::BitString {
                            unused_bits: 4,
                            data: vec![0]
                        })
                    )
                    .unwrap()
                ),
                "each reference retains its own generation"
            );
        }
    }
}

#[tokio::test]
async fn cov_identity_held_initial_completion_cannot_overwrite_renewal_or_recreation() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            for change in [Change::Renew, Change::Recreate, Change::Remove] {
                stale_completion(
                    true,
                    kind,
                    confirmed,
                    change,
                    PropertyIdentifier::PRESENT_VALUE,
                )
                .await;
            }
        }
    }
}

#[tokio::test]
async fn cov_identity_held_fanout_completion_fences_each_reference_and_mode() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            for change in [Change::Renew, Change::Recreate, Change::Remove] {
                stale_completion(
                    false,
                    kind,
                    confirmed,
                    change,
                    PropertyIdentifier::PRESENT_VALUE,
                )
                .await;
            }
        }
    }
}

mod lifetime;

mod property_samples;

mod sample_contract;

mod status_flags;

mod status_contract;

mod route_migration;

mod recipient_route;

mod value_source;

mod value_source_contract;

mod completion_order;

mod noncommandable_value_source;
