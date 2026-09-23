//! Post-start validation failure owns cleanup even when the caller cancels it.
use super::*;
use tokio::sync::{Notify, Semaphore};

struct BlockingStop {
    inner: ObservedTransport,
    entered: Arc<Notify>,
    gate: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
}
impl Drop for BlockingStop {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl TransportPort for BlockingStop {
    fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        self.inner.bip_broadcast_endpoint()
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.entered.notify_one();
        self.gate.acquire().await.unwrap().forget();
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

#[tokio::test]
async fn canceled_failed_start_retains_cleanup_until_stop_or_drop() {
    for (finish_by_stop, after_install) in
        [(true, false), (false, false), (true, true), (false, true)]
    {
        let (inner, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
        let observed = Arc::new(Observations::default());
        let entered = Arc::new(Notify::new());
        let gate = Arc::new(Semaphore::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let transport = BlockingStop {
            inner: ObservedTransport {
                inner,
                observed: observed.clone(),
            },
            entered: entered.clone(),
            gate: gate.clone(),
            drops: drops.clone(),
        };
        let mut db = database();
        if !after_install {
            db.get_mut(&oid(ObjectType::DEVICE, 123))
                .unwrap()
                .device_authority_internal()
                .unwrap()
                .provision_audit_recipient(bacnet_types::constructed::BACnetRecipient::Address(
                    bacnet_types::constructed::BACnetAddress {
                        network_number: 0,
                        mac_address: bacnet_types::MacAddr::from_slice(&[
                            192, 168, 1, 255, 0xBA, 0xC0,
                        ]),
                    },
                ))
                .unwrap();
        }
        let config = if after_install {
            SessionConfig {
                max_apdu_length: 1,
                ..SessionConfig::default()
            }
        } else {
            SessionConfig::default()
        };
        let mut session = EndpointSession::new(transport, SessionRole::ClientOnly, config)
            .unwrap()
            .with_database(db)
            .with_source_audit_reporter(selected());
        {
            let start = session.start();
            tokio::pin!(start);
            tokio::select! {
                result = &mut start => panic!("cleanup returned before gate: {result:?}"),
                () = entered.notified() => {}
            }
        }
        assert_eq!(
            session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopping as u8
        );
        assert!(!session.is_running());
        assert!(session.client().is_none() && session.server().is_none());
        assert_eq!(session.active_leases(), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(session.start().await.is_err());
        assert_eq!(session.source_recipient.is_some(), after_install);
        {
            let mut db = session.database.as_ref().unwrap().write().await;
            assert!(!source(&db, selected()));
            if after_install {
                assert!(db.remove(&selected()).is_err());
            }
        }
        if finish_by_stop {
            gate.add_permits(1);
            session.stop().await.unwrap();
            assert_eq!(
                session.lifecycle.load(Ordering::Acquire),
                Lifecycle::Stopped as u8
            );
            assert_eq!(observed.stops.load(Ordering::SeqCst), 1);
        }
        let retained_db = session.database.as_ref().unwrap().clone();
        drop(session);
        tokio::time::timeout(Duration::from_secs(2), async {
            while drops.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut db = retained_db.write().await;
        assert!(!source(&db, selected()));
        assert!(db.remove(&selected()).is_ok());
        assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn post_profile_requester_failure_uninstalls_source_owner_and_releases_database() {
    let (inner, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let observed = Arc::new(Observations::default());
    let mut session = EndpointSession::new(
        ObservedTransport {
            inner,
            observed: observed.clone(),
        },
        SessionRole::ClientOnly,
        SessionConfig {
            max_apdu_length: 1,
            ..SessionConfig::default()
        },
    )
    .unwrap()
    .with_database(database())
    .with_source_audit_reporter(selected());
    assert!(session.start().await.is_err());
    assert_eq!(observed.starts.load(Ordering::SeqCst), 1);
    assert_eq!(observed.stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        session.lifecycle.load(Ordering::Acquire),
        Lifecycle::Stopped as u8
    );
    assert!(session.source_recipient.is_none() && session.source_read.is_none());
    assert!(session.notifications.is_none());
    let mut db = session.database.as_ref().unwrap().write().await;
    assert!(!source(&db, selected()));
    assert!(!db
        .get(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .property_list()
        .contains(&PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT));
    assert!(db.remove(&selected()).unwrap().is_some());
    assert!(db.remove(&oid(ObjectType::DEVICE, 123)).unwrap().is_some());
}
