//! A server on a router's application port (#2601): an unconfirmed event
//! notification to a local-broadcast recipient goes to each of the router's
//! networks once, an isolated port's included, and no further.

use super::event_notifications_tests::local_broadcast_destination;
use super::*;
use crate::server::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_network::router::{BACnetRouter, RouterApplicationPorts, RouterOptions, RouterPort};
use bacnet_network::router_table::PortId;
use bacnet_npdu::Mac;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::EventStateChange;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState, EventType};

/// The APDUs on `log`, past the router's own network messages.
fn apdus(log: &SendLog) -> Vec<SentFrame> {
    log.frames()
        .into_iter()
        .filter(|frame| frame.npdu.get(1).is_some_and(|control| control & 0x80 == 0))
        .collect()
}

/// A database whose AnalogInput 1 reports to Notification Class 0, whose
/// one recipient is the local broadcast.
fn database() -> ObjectDatabase {
    let mut db = clocked_test_database();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    nc.add_destination(local_broadcast_destination()).unwrap();
    db.add(Box::new(nc)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Router device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        None,
    )
    .unwrap();
    db.add(Box::new(ai)).unwrap();
    db
}

/// The router on network 1 (MAC 1) and isolated network 2 (MAC 2), with
/// network 9 learned beyond network 1 through the router at MAC 7. The
/// server's share of the application entity sends one TO_OFFNORMAL
/// notification: network 1 and network 2 each see one copy, as that link's
/// broadcast with no DNET, and nothing goes toward network 9.
#[tokio::test(start_paused = true)]
async fn a_local_broadcast_notification_reaches_each_attached_network_once() {
    // Each link stays open while the test holds its sender: a port whose
    // link closes ends its dispatch, and the router with its last one.
    let (links, _inbound): (Vec<TestTransport>, Vec<_>) = (1..=2)
        .map(|mac| {
            let (inbound, rx) = tokio::sync::mpsc::channel(4);
            let link = TestTransport::builder()
                .local_mac(&[mac])
                .inbound(rx)
                .build();
            (link, inbound)
        })
        .unzip();
    let logs: Vec<SendLog> = links.iter().map(TestTransport::sent).collect();
    let ports = links
        .into_iter()
        .zip([1, 2])
        .map(|(link, network)| RouterPort::new(link, network).isolated(network == 2))
        .collect();
    let started = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let (mut router, RouterApplicationPorts { server, .. }, _) = started.into_application_ports();
    let now = router.now();
    assert!(
        router
            .table()
            .add_learned(9, PortId(0), Mac::from_array([7]), now)
    );

    let network = Arc::new(NetworkLayer::new(server));
    let db = Arc::new(RwLock::new(database()));
    let comm_state = Arc::new(CommState::default());
    let suppressions = Arc::default();
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));
    let notifications = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(
        super::device_bindings::DeviceBindingTable::new(),
    ));
    let device = crate::server::event_delivery::fixture_device(&db).await;
    let ctx = EventDelivery {
        db: &db,
        network: &network,
        comm_state: &comm_state,
        learned_routers: &learned_routers,
        notification_transactions: &notifications,
        device_bindings: &device_bindings,
        suppressions: &suppressions,
        retry_timeout_ms: 1000,
        local_apdu_capacity: 1476,
        device,
    };
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    BACnetServer::<bacnet_network::router::RouterApplicationPort>::build_and_send_event_notification_with_bindings(
        &ctx,
        &oid,
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
    )
    .await;
    // The router's port senders take their copies from its queues.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    for (log, network) in logs.iter().zip([1, 2]) {
        let sent = apdus(log);
        let [copy] = sent.as_slice() else {
            panic!("network {network}: one APDU, not {sent:?}")
        };
        assert!(copy.broadcast, "network {network}: the link's broadcast");
        assert!(copy.decode_npdu().destination.is_none(), "no DNET");
        assert!(matches!(
            copy.apdu(),
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
        ));
        assert!(log.unicasts().is_empty(), "nothing toward network 9");
    }
    router.stop().await.unwrap();
}
