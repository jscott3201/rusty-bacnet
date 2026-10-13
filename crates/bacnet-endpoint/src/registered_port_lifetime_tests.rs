//! Controlled real B/IP cleanup proves leases outlive cancelled caller waiters.
use super::*;
use bacnet_transport::{bip::BipTransport, port::ReceivedNpdu};
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{
        atomic::{AtomicBool, AtomicU8},
        Mutex as SyncMutex, Weak,
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};

struct Control {
    stop_entered: Notify,
    stop_finished: Notify,
    release: Semaphore,
    started: Notify,
    start_release: Semaphore,
    hold_start: AtomicBool,
    mismatch: AtomicBool,
    stop_outcome: AtomicU8,
    lease: SyncMutex<Weak<()>>,
}
impl Control {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stop_entered: Notify::new(),
            stop_finished: Notify::new(),
            release: Semaphore::new(0),
            started: Notify::new(),
            start_release: Semaphore::new(0),
            hold_start: AtomicBool::new(false),
            mismatch: AtomicBool::new(false),
            stop_outcome: AtomicU8::new(0),
            lease: SyncMutex::new(Weak::new()),
        })
    }
    fn alive(&self) -> bool {
        self.lease.lock().unwrap().strong_count() != 0
    }
}
struct Controlled {
    bip: BipTransport,
    control: Arc<Control>,
    bound: bool,
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
}
impl Controlled {
    fn new(control: Arc<Control>) -> Self {
        Self {
            bip: BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
            control,
            bound: false,
            incoming: None,
        }
    }
}
impl TransportPort for Controlled {
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        self.bip.bip_broadcast_endpoint()
    }
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        self.bip.supports_local_nonrouter_number_controls()
    }
    fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        if self.bound && self.control.mismatch.load(Ordering::SeqCst) {
            None
        } else {
            self.bip.bip_port()
        }
    }
    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        *self.control.lease.lock().unwrap() = Arc::downgrade(&lease);
        self.bip.retain_network_port_lease_internal(lease)
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let receiver = self.bip.start().await?;
        self.bound = true;
        self.control.started.notify_one();
        if self.control.hold_start.load(Ordering::SeqCst) {
            self.control.start_release.acquire().await.unwrap().forget();
        }
        Ok(self.incoming.take().unwrap_or(receiver))
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.control.stop_entered.notify_one();
        self.control.release.acquire().await.unwrap().forget();
        match self.control.stop_outcome.load(Ordering::SeqCst) {
            1 => return Err(Error::Encoding("injected registered stop failure".into())),
            2 => panic!("injected registered stop panic"),
            _ => {}
        }
        self.bip.stop().await?;
        self.control.stop_finished.notify_one();
        Ok(())
    }
    fn abort(&mut self) {
        self.bip.abort();
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.bip.local_receive_apdu_capacity()
    }

    fn egress_apdu_limit(&self) -> u16 {
        // Deliberately asymmetric wrapper: port publication must use local1476.
        480
    }

    fn local_mac(&self) -> &[u8] {
        self.bip.local_mac()
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.bip.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.bip.send_broadcast(npdu).await
    }
}
fn port() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}
fn identity() -> crate::DeviceIdentity {
    crate::DeviceIdentity::new(785, 555)
        .unwrap()
        .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)
        .unwrap()
}
async fn wait(notify: &Notify) {
    tokio::time::timeout(Duration::from_secs(3), notify.notified())
        .await
        .unwrap();
}
async fn assert_protected(db: &Arc<RwLock<ObjectDatabase>>) {
    let mut db = db.write().await;
    assert!(db.remove(&port()).is_err());
    assert!(db
        .with_object_adapter(&port(), |_| panic!("protected callback"))
        .is_err());
    let replacement = bacnet_objects::network_port::NetworkPortObject::new_bip(
        1,
        "replacement",
        Default::default(),
    )
    .unwrap();
    assert!(db.add(Box::new(replacement)).is_err());
    assert!(db
        .get_mut(&port())
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None
        )
        .is_err());
}
async fn server(control: Arc<Control>) -> bacnet_server::server::BACnetServer<Controlled> {
    let id = identity();
    let mut config = id.server_config();
    config.registered_network_port = Some(port());
    bacnet_server::server::BACnetServer::start(
        config,
        id.build_database().unwrap(),
        Controlled::new(control),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn registered_port_server_cancelled_stop_then_drop_retains_cleanup_lease() {
    let control = Control::new();
    let mut server = server(control.clone()).await;
    let db = server.database().clone();
    {
        let stop = server.stop();
        tokio::pin!(stop);
        tokio::select! { result = &mut stop => panic!("cleanup unexpectedly ended: {result:?}"), _ = wait(&control.stop_entered) => {} }
    }
    assert_protected(&db).await;
    drop(server);
    assert_protected(&db).await;
    assert!(control.alive());
    control.release.add_permits(1);
    wait(&control.stop_finished).await;
    assert!(!control.alive());
    assert!(db.write().await.remove(&port()).unwrap().is_some());
}

#[tokio::test]
async fn registered_port_server_cleanup_error_keeps_lease_for_retry() {
    let control = Control::new();
    let mut server = server(control.clone()).await;
    let db = server.database().clone();
    control.stop_outcome.store(1, Ordering::SeqCst);
    control.release.add_permits(1);
    assert!(server.stop().await.is_err());
    assert_protected(&db).await;
    assert!(control.alive());
    control.stop_outcome.store(0, Ordering::SeqCst);
    control.release.add_permits(1);
    server.stop().await.unwrap();
    assert!(!control.alive());
    assert!(db.write().await.remove(&port()).unwrap().is_some());
}

#[tokio::test]
async fn registered_port_failed_postbind_start_cleanup_survives_caller_cancel() {
    let control = Control::new();
    control.mismatch.store(true, Ordering::SeqCst);
    let id = identity();
    let mut config = id.server_config();
    config.registered_network_port = Some(port());
    let task = tokio::spawn(bacnet_server::server::BACnetServer::start(
        config,
        id.build_database().unwrap(),
        Controlled::new(control.clone()),
    ));
    wait(&control.stop_entered).await;
    assert!(control.alive());
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(control.alive());
    control.release.add_permits(1);
    wait(&control.stop_finished).await;
    assert!(!control.alive());
}

#[tokio::test]
async fn registered_port_endpoint_cancelled_start_and_stop_join_same_cleanup() {
    let control = Control::new();
    control.hold_start.store(true, Ordering::SeqCst);
    let id = identity();
    let mut session = EndpointSession::new(
        Controlled::new(control.clone()),
        SessionRole::ServerOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(id.build_database().unwrap())
    .with_identity(id)
    .with_registered_network_port(port());
    let db = session.database.as_ref().unwrap().clone();
    {
        let start = session.start();
        tokio::pin!(start);
        tokio::select! { result = &mut start => panic!("held start returned: {result:?}"), _ = wait(&control.started) => {} }
    }
    assert!(session.bip_local_address().is_none());
    assert_protected(&db).await;
    {
        let stop = session.stop();
        tokio::pin!(stop);
        tokio::select! { result = &mut stop => panic!("held stop returned: {result:?}"), _ = wait(&control.stop_entered) => {} }
    }
    assert_protected(&db).await;
    control.release.add_permits(1);
    session.stop().await.unwrap();
    assert!(!control.alive());
    assert!(db.write().await.remove(&port()).unwrap().is_some());
}

#[tokio::test]
async fn registered_port_admitted_read_survives_independent_ingress_end() {
    for direct in [false, true] {
        let control = Control::new();
        let (incoming, receiver) = mpsc::channel(1);
        let mut transport = Controlled::new(control.clone());
        transport.incoming = Some(receiver);
        let id = identity();
        let mut session =
            EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())
                .unwrap()
                .with_database(id.build_database().unwrap())
                .with_identity(id)
                .with_registered_network_port(port());
        session.start().await.unwrap();
        let idle_handle = session.cloned_server_handle().unwrap();
        let db = session.database.as_ref().unwrap().clone();
        let mut locked = db.write().await;
        let baseline = control.lease.lock().unwrap().strong_count();
        // Real encoded RP for NETWORK_PORT:4194303 Object_Identifier; block its DB read.
        let npdu =
            bytes::Bytes::from_static(&[1, 4, 0, 5, 1, 12, 12, 0x0e, 0x3f, 0xff, 0xff, 25, 75]);
        let mac = bacnet_types::MacAddr::from_slice(&[127, 0, 0, 1, 0xba, 0xc0]);
        let direct_call = if direct {
            let handle = idle_handle.clone();
            Some(tokio::spawn(async move {
                handle
                    .handle_inbound(ReceivedApdu {
                        direct_response: None,
                        apdu: npdu.slice(2..),
                        source_mac: mac,
                        ingress_network: None,
                        source_network: None,
                        link_layer_group: false,
                        is_group: false,
                        global_broadcast: false,
                        data_attributes: vec![],
                        provenance: bacnet_transport::port::TransportProvenance::unverified(),
                        reply_tx: None,
                    })
                    .await
            }))
        } else {
            incoming
                .send(ReceivedNpdu::unverified(npdu, mac, false, vec![], None))
                .await
                .unwrap();
            None
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            // Per-call strong acquisition is an exact admission observation, not a delay.
            while control.lease.lock().unwrap().strong_count() <= baseline {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(incoming);
        wait(&control.stop_entered).await;
        control.release.add_permits(1);
        wait(&control.stop_finished).await;
        assert!(session.bip_local_address().is_none());
        // UDP is stopped; the already-admitted reader still protects object identity.
        assert!(locked.remove(&port()).is_err());
        assert!(locked
            .get_mut(&port())
            .unwrap()
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None
            )
            .is_err());
        drop(locked);
        if let Some(call) = direct_call {
            assert!(call.await.unwrap().is_err());
        }
        session.stop().await.unwrap();
        assert!(!control.alive());
        assert!(db.write().await.remove(&port()).unwrap().is_some());
        // Retaining an inactive public role does not reserve the object indefinitely.
        assert!(!idle_handle.is_session_alive());
    }
}

#[tokio::test]
async fn registered_port_endpoint_cleanup_error_is_reported() {
    let control = Control::new();
    let id = identity();
    let mut session = EndpointSession::new(
        Controlled::new(control.clone()),
        SessionRole::ServerOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(id.build_database().unwrap())
    .with_identity(id)
    .with_registered_network_port(port());
    session.start().await.unwrap();
    control.stop_outcome.store(1, Ordering::SeqCst);
    control.release.add_permits(1);
    assert!(session.stop().await.is_err());
    assert!(session.bip_local_address().is_none());
    tokio::time::timeout(Duration::from_secs(3), async {
        while control.alive() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(session
        .database
        .as_ref()
        .unwrap()
        .write()
        .await
        .remove(&port())
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn registered_port_server_cleanup_panic_releases_only_with_final_abort() {
    let control = Control::new();
    let mut server = server(control.clone()).await;
    let db = server.database().clone();
    control.stop_outcome.store(2, Ordering::SeqCst);
    control.release.add_permits(1);
    assert!(server.stop().await.is_err());
    assert!(server.stop().await.is_err());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let Some(hold) = control.lease.lock().unwrap().upgrade() else {
                break;
            };
            assert_protected(&db).await;
            drop(hold);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(db.write().await.remove(&port()).unwrap().is_some());
}

#[tokio::test]
async fn registered_port_active_endpoint_cancelled_stop_retains_and_then_releases() {
    let control = Control::new();
    let id = identity();
    let mut session = EndpointSession::new(
        Controlled::new(control.clone()),
        SessionRole::ServerOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(id.build_database().unwrap())
    .with_identity(id)
    .with_registered_network_port(port());
    session.start().await.unwrap();
    let db = session.database.as_ref().unwrap().clone();
    let idle = session.cloned_server_handle().unwrap();
    {
        let stop = session.stop();
        tokio::pin!(stop);
        tokio::select! { result=&mut stop => panic!("held stop returned: {result:?}"), _=wait(&control.stop_entered)=>{} }
    }
    assert!(session.bip_local_address().is_none());
    assert_protected(&db).await;
    control.release.add_permits(1);
    session.stop().await.unwrap();
    assert!(!idle.is_session_alive());
    assert!(!control.alive());
    assert!(db.write().await.remove(&port()).unwrap().is_some());
}

#[tokio::test]
async fn both_registered_bip_snapshots_use_local_capacity_not_smaller_egress() {
    let control = Control::new();
    let mut full = server(control.clone()).await;
    assert_eq!(
        full.database()
            .read()
            .await
            .get(&port())
            .unwrap()
            .read_property(PropertyIdentifier::APDU_LENGTH, None)
            .unwrap(),
        PropertyValue::Unsigned(1476)
    );
    control.release.add_permits(1);
    full.stop().await.unwrap();

    let control = Control::new();
    let id = identity();
    let mut session = EndpointSession::new(
        Controlled::new(control.clone()),
        SessionRole::ServerOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(id.build_database().unwrap())
    .with_identity(id)
    .with_registered_network_port(port());
    session.start().await.unwrap();
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .get(&port())
            .unwrap()
            .read_property(PropertyIdentifier::APDU_LENGTH, None)
            .unwrap(),
        PropertyValue::Unsigned(1476)
    );
    control.release.add_permits(1);
    session.stop().await.unwrap();
}
