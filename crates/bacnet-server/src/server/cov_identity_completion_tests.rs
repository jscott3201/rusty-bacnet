use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::analog::AnalogValueObject;
use bytes::Bytes;
use std::sync::Mutex as StdMutex;
use tokio::sync::Notify;

struct HeldTransport {
    sent: Arc<StdMutex<Vec<Bytes>>>,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
    hold: bool,
}
impl TransportPort for HeldTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        Ok(mpsc::channel(1).1)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, npdu: &[u8], _: &[u8]) -> Result<(), Error> {
        self.sent.lock().unwrap().push(Bytes::copy_from_slice(npdu));
        self.entered.notify_one();
        if self.hold {
            self.release.acquire().await.unwrap().forget();
        }
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xba, 0xc0]
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
        expires_at: None,
        last_notified_value: Some(1.0),
        monitored_property: Some(property),
        monitored_property_array_index: None,
        cov_increment: Some(0.1),
        notification_kind: kind,
        timestamped: false,
    }
}

struct Fixture {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<HeldTransport>>,
    table: Arc<RwLock<CovSubscriptionTable>>,
    permits: Arc<Semaphore>,
    transactions: Arc<NotificationTransactions>,
    comm: Arc<AtomicU8>,
    config: ServerConfig,
    sent: Arc<StdMutex<Vec<Bytes>>>,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
}
impl Fixture {
    fn new(hold: bool) -> Self {
        let mut db = clocked_test_database();
        let mut av = AnalogValueObject::new(3, "value", 95).unwrap();
        av.set_present_value(10.0);
        db.add(Box::new(av)).unwrap();
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Semaphore::new(0));
        Self {
            db: Arc::new(RwLock::new(db)),
            network: Arc::new(NetworkLayer::new(HeldTransport {
                sent: sent.clone(),
                entered: entered.clone(),
                release: release.clone(),
                hold,
            })),
            table: Arc::new(RwLock::new(CovSubscriptionTable::new())),
            permits: Arc::new(Semaphore::new(8)),
            transactions: NotificationTransactions::new(),
            comm: Arc::new(AtomicU8::new(0)),
            config: ServerConfig::default(),
            sent,
            entered,
            release,
        }
    }
    async fn fire(&self, initial: bool, snapshots: &[CovSubscriptionSnapshot]) {
        if !initial {
            BACnetServer::<HeldTransport>::fire_cov_notifications(
                &self.db,
                &self.network,
                &self.table,
                &self.permits,
                &self.transactions,
                &self.comm,
                &self.config,
                &object(),
            )
            .await;
        } else if snapshots[0].notification_kind == CovNotificationKind::Single {
            BACnetServer::<HeldTransport>::fire_initial_cov_notification(
                &self.db,
                &self.network,
                &self.table,
                &self.permits,
                &self.transactions,
                &self.comm,
                &self.config,
                &snapshots[0],
            )
            .await;
        } else {
            BACnetServer::<HeldTransport>::fire_initial_cov_notification_multiple(
                &self.db,
                &self.network,
                &self.table,
                &self.permits,
                &self.transactions,
                &self.comm,
                &self.config,
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
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice
                })
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
) {
    let fixture = Fixture::new(!confirmed);
    let original = proposal(kind, confirmed, PropertyIdentifier::PRESENT_VALUE);
    let mut snapshots = vec![fixture
        .table
        .write()
        .await
        .subscribe(original.clone())
        .unwrap()];
    if kind == CovNotificationKind::Multiple {
        snapshots.push(
            fixture
                .table
                .write()
                .await
                .subscribe(proposal(kind, confirmed, PropertyIdentifier::COV_INCREMENT))
                .unwrap(),
        );
    }
    // Confirmed baseline is assigned at admission, before transport send. Hold the
    // actual object-read boundary after the snapshot was captured, not a mock setter.
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
            renewed.last_notified_value = Some(99.0);
            table.subscribe(renewed).unwrap();
        }
    }
    drop(db_guard);
    fixture.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), work)
        .await
        .unwrap();
    {
        let table = fixture.table.read().await;
        if matches!(change, Change::Remove) {
            assert!(table.get_subscription(snapshots[0].key()).is_none());
        } else {
            assert_eq!(table.get_subscription(snapshots[0].key()).unwrap().last_notified_value,Some(99.0),"stale completion initial={initial} kind={kind:?} confirmed={confirmed} change={change:?}");
        }
        if kind == CovNotificationKind::Multiple {
            assert_eq!(
                table
                    .get_subscription(snapshots[1].key())
                    .unwrap()
                    .last_notified_value,
                Some(10.0),
                "each reference retains its own generation"
            );
        }
    }
    let admitted = confirmed && kind == CovNotificationKind::Multiple;
    if confirmed && !admitted {
        assert!(
            fixture.sent.lock().unwrap().is_empty(),
            "late stale Single ownership must suppress admission"
        );
        assert_eq!(fixture.transactions.active_count(), 0);
    }
    fixture.finish(admitted).await;
}

#[tokio::test]
async fn cov_identity_held_initial_completion_cannot_overwrite_renewal_or_recreation() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            for change in [Change::Renew, Change::Recreate, Change::Remove] {
                stale_completion(true, kind, confirmed, change).await;
            }
        }
    }
}

#[tokio::test]
async fn cov_identity_held_fanout_completion_fences_each_reference_and_mode() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            for change in [Change::Renew, Change::Recreate, Change::Remove] {
                stale_completion(false, kind, confirmed, change).await;
            }
        }
    }
}

mod lifetime;
