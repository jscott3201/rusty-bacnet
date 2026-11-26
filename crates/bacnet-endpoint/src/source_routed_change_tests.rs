//! Routed source Audit bindings seen from the link itself (#2371): the
//! session's number changing to the binding's network while a record is in
//! flight and a lost-record count is queued, and a router learned from the
//! answer to the session's own router query. The peer is a bare loopback
//! end that reads and writes NPDUs, so it sees every DNET and link
//! destination.
use super::*;
use bacnet_encoding::npdu::{Npdu, decode_npdu};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, TestTransport};

/// The binding's network, which the session's own number becomes.
const BOUND: u16 = 7;

/// A loopback link whose node takes the local Number controls, as B/IP,
/// SC and MS/TP do, so the session reads network messages. Each send takes
/// a permit from its gate, so a test can hold sends in the transport.
fn control_link(inner: LoopbackTransport, gate: Arc<tokio::sync::Semaphore>) -> TestTransport {
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .number_controls()
        .on_send(move |_| {
            let gate = Arc::clone(&gate);
            async move {
                gate.acquire().await.unwrap().forget();
                Ok(())
            }
        })
        .build()
}

/// The session at `SESSION`, its Reporter confirmed and bound to Device
/// 999 as station 5 on network 7, and the bare peer at station 5 on the
/// same link.
struct Link {
    session: EndpointSession<TestTransport>,
    peer: LoopbackTransport,
    rx: tokio::sync::mpsc::Receiver<ReceivedNpdu>,
    /// The session's sends' gate, open until a test closes it.
    gate: Arc<tokio::sync::Semaphore>,
    /// The session's router query for network 7 was seen (#2831).
    queried: bool,
}

impl Link {
    async fn start(operations: &[AuditOperation]) -> Self {
        let (near, mut peer) = LoopbackTransport::pair(SESSION.to_vec(), STATION.to_vec());
        let mut flags = AuditOperationFlags::empty();
        for operation in operations {
            flags.insert(*operation);
        }
        let mut db = provisioned();
        db.get_mut(&selected())
            .unwrap()
            .audit_reporter_mut()
            .unwrap()
            .configure(
                AuditLevel::AUDIT_ALL,
                flags,
                true,
                None,
                BACnetPriorityFilter::empty(),
                None,
            )
            .unwrap();
        let gate = Arc::new(tokio::sync::Semaphore::new(1 << 20));
        let mut session = EndpointSession::new(
            control_link(near, Arc::clone(&gate)),
            SessionRole::ClientOnly,
            SessionConfig::default(),
        )
        .unwrap()
        .with_database(db)
        .with_source_audit_routed_device_binding(
            oid(ObjectType::DEVICE, 999),
            BOUND,
            MacAddr::from_slice(&STATION),
        )
        .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        let rx = peer.start().await.unwrap();
        Self {
            session,
            peer,
            rx,
            gate,
            queried: false,
        }
    }

    /// The routers the session's source Audit learns.
    fn routers(&self) -> Arc<crate::source_audit::routers::SourceRouters> {
        Arc::clone(
            self.session
                .source_recipient
                .as_ref()
                .unwrap()
                .routes
                .routers(),
        )
    }

    /// The next APDU-carrying NPDU the session sent, skipping network
    /// messages (noting its router query): whether it went to a group, its
    /// NPDU and its record.
    async fn record(&mut self) -> (bool, Npdu, BACnetAuditNotification, u8) {
        loop {
            let received = timeout(WAIT, self.rx.recv()).await.unwrap().unwrap();
            let npdu = decode_npdu(received.npdu.clone()).unwrap();
            if npdu.is_network_message() {
                self.queried |= is_router_query(received.link_layer_group, &npdu);
                continue;
            }
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload.clone()).unwrap() else {
                panic!("a confirmed record");
            };
            let mut records =
                bacnet_services::audit::AuditNotificationRequest::decode(&request.service_request)
                    .unwrap();
            let record = records.notifications.remove(0);
            return (received.link_layer_group, npdu, record, request.invoke_id);
        }
    }

    /// Send `npdu` bytes from the peer: to the session, or to every node.
    async fn send(&self, npdu: &[u8], broadcast: bool) {
        if broadcast {
            self.peer.send_broadcast(npdu).await.unwrap();
        } else {
            self.peer.send_unicast(npdu, &SESSION).await.unwrap();
        }
    }

    /// Acknowledge a record straight from the station, with no SNET.
    async fn acknowledge(&self, invoke_id: u8) {
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            }),
        )
        .unwrap();
        let npdu = [&[0x01, 0x00][..], &apdu].concat();
        self.send(&npdu, false).await;
    }

    fn number(&self) -> Option<u16> {
        self.session
            .egress
            .as_ref()
            .unwrap()
            .local_network_number()
            .get()
    }
}

/// A Network-Number-Is for `network`, configured (Clause 6.4.20).
fn number_is(network: u16) -> Vec<u8> {
    let [high, low] = network.to_be_bytes();
    vec![0x01, 0x80, 0x13, high, low, 1]
}

/// The session learns that network 7, the binding's, is its own while one
/// confirmed record is in flight to it through the router and a count of
/// lost records is queued. The station, now on this link, answers the
/// record directly, which completes it. The count then goes to the
/// station's MAC with no DNET, which a non-routing node would otherwise
/// drop, and its direct answer completes it too, clearing the failure.
#[tokio::test(start_paused = true)]
async fn a_number_change_takes_routed_records_in_flight_and_queued_as_local() {
    let operations = [
        AuditOperation::DEVICE_RESET,
        AuditOperation::AUDITING_FAILURE,
    ];
    let mut link = Link::start(&operations).await;
    link.session.report_device_reset_local().await.unwrap();
    let (group, npdu, record, in_flight) = link.record().await;
    assert!(group, "no router is known: the link's broadcast MAC");
    assert_eq!(npdu.destination.unwrap().network, BOUND);
    assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
    // Every other admission permit held: the next record is lost, counted.
    let owner = Arc::clone(link.session.notifications.as_ref().unwrap());
    let permits: Vec<_> = std::iter::from_fn(|| owner.try_admit_audit().ok()).collect();
    link.session.report_device_reset_local().await.unwrap();
    link.send(&number_is(BOUND), true).await;
    eventually(WAIT, "the session's number is 7", async || {
        link.number() == Some(BOUND)
    })
    .await;
    link.acknowledge(in_flight).await;
    drop(permits);
    let (group, npdu, summary, invoke_id) = link.record().await;
    assert!(!group, "sent to the station's MAC");
    assert_eq!(npdu.destination, None, "sent with no DNET");
    assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
    link.acknowledge(invoke_id).await;
    // Past the records' deadline: an answer that matched nothing would
    // have failed its record by now.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        reliability(&link.session).await,
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    assert_eq!(link.session.coordinator.active_count().unwrap(), 0);
    link.session.stop().await.unwrap();
}

/// Whether `npdu` is the session's router query (Who-Is-Router-To-Network,
/// Clause 6.4.1) for network 7, by broadcast.
fn is_router_query(group: bool, npdu: &Npdu) -> bool {
    group
        && npdu.message_type == Some(bacnet_npdu::NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
        && npdu.payload.as_ref() == BOUND.to_be_bytes()
}

impl Link {
    /// Wait for the session's router query for network 7, answer it as the
    /// one router to that network, and wait until the session takes it.
    async fn answer_router_query(&mut self) {
        while !self.queried {
            let received = timeout(WAIT, self.rx.recv()).await.unwrap().unwrap();
            let npdu = decode_npdu(received.npdu.clone()).unwrap();
            self.queried = is_router_query(received.link_layer_group, &npdu);
        }
        // I-Am-Router-To-Network (Clause 6.4.2) for network 7.
        self.send(&[0x01, 0x80, 0x01, 0, BOUND as u8], true).await;
        // Decided at the end of the query's wait, the APDU timeout.
        let routers = self.routers();
        eventually(
            Duration::from_secs(7),
            "the answer teaches the router",
            async || routers.router(BOUND).is_some(),
        )
        .await;
        assert_eq!(routers.router(BOUND).as_deref(), Some(&STATION[..]));
    }
}

/// A router announcement (I-Am-Router-To-Network) that the session did not
/// ask for teaches nothing, whatever it lists. A record that goes by
/// broadcast for want of a router has the session ask its router query, and
/// the one router answering it is learned (#2831): the next record goes to
/// it as a unicast, with DNET 7.
#[tokio::test(start_paused = true)]
async fn only_the_answer_to_the_router_query_teaches_its_sender() {
    let mut link = Link::start(&[AuditOperation::DEVICE_RESET]).await;
    let routers = link.routers();
    // I-Am-Router-To-Network for networks 8 and 7, before anything asked.
    link.send(&[0x01, 0x80, 0x01, 0, 8, 0, BOUND as u8], true)
        .await;
    tokio::time::sleep(WAIT).await;
    assert_eq!(
        routers.router(BOUND),
        None,
        "an unasked announcement taught it"
    );
    link.session.report_device_reset_local().await.unwrap();
    let (group, npdu, record, invoke_id) = link.record().await;
    assert!(group, "no router is known: the link's broadcast MAC");
    assert_eq!(npdu.destination.unwrap().network, BOUND);
    assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
    link.acknowledge(invoke_id).await;
    link.answer_router_query().await;
    link.session.report_device_reset_local().await.unwrap();
    let (group, npdu, record, _) = link.record().await;
    assert!(!group, "sent to the router's MAC");
    assert_eq!(npdu.destination.unwrap().network, BOUND);
    assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
    link.session.stop().await.unwrap();
}

/// A routed record that expires in the session's own egress queue never
/// went out on the link, so it fails, setting COMMUNICATION_FAILURE, but
/// the router it would have gone through stays learned.
#[tokio::test(start_paused = true)]
async fn a_record_expired_in_the_local_queue_keeps_its_router() {
    let mut link = Link::start(&[AuditOperation::DEVICE_RESET]).await;
    let routers = link.routers();
    link.session.report_device_reset_local().await.unwrap();
    let (_, _, _, invoke_id) = link.record().await;
    link.acknowledge(invoke_id).await;
    link.answer_router_query().await;
    // One unrelated send enters the transport and waits at the closed
    // gate, so the record queues behind it until its deadline.
    link.gate.forget_permits(usize::MAX);
    let blocker = link
        .session
        .egress
        .as_ref()
        .unwrap()
        .admit_apdu(
            vec![0x10, 8],
            EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(&STATION),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
            None,
        )
        .unwrap();
    link.session.report_device_reset_local().await.unwrap();
    let failed = Reliability::COMMUNICATION_FAILURE.to_raw();
    eventually(
        Duration::from_secs(10),
        "the record expires unsent",
        async || reliability(&link.session).await == failed,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(routers.router(BOUND).as_deref(), Some(&STATION[..]));
    link.gate.add_permits(1 << 20);
    blocker.complete().await.result.unwrap();
    link.session.stop().await.unwrap();
}
