//! A server on a router's application port (#2601) takes the same check
//! (#2732): such a server has no network of its own, so every recipient
//! with a network number stays remote. A confirmed recipient at the MS/TP
//! broadcast form, on a directly attached network or on one learned beyond
//! it, gets nothing on either link and counts once; an unconfirmed one at
//! the learned network's form goes to the router there.

use super::*;
use crate::server::event_delivery::{EventDelivery, fixture_device};
use crate::server::test_transport::SentFrame;
use bacnet_network::router::{BACnetRouter, RouterApplicationPorts, RouterOptions, RouterPort};
use bacnet_network::router_table::PortId;
use bacnet_npdu::Mac;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState, EventType};

/// The APDUs on `log`, past the router's own network messages.
fn apdus(log: &SendLog) -> Vec<SentFrame> {
    log.frames()
        .into_iter()
        .filter(|frame| frame.npdu.get(1).is_some_and(|control| control & 0x80 == 0))
        .collect()
}

/// AnalogInput 1, reporting to Notification Class 0, which holds
/// `destinations`.
fn database(destinations: Vec<BACnetDestination>) -> ObjectDatabase {
    let mut db = crate::server::clocked_test_database();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    for destination in destinations {
        nc.add_destination(destination).unwrap();
    }
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

/// The router on network 1 (MAC 1) and network 2 (MAC 2), with network 9
/// learned beyond network 1 through the router at MAC 7.
#[tokio::test(start_paused = true)]
async fn a_router_application_server_sends_no_confirmed_notification_to_a_remote_broadcast() {
    let (links, _inbound): (Vec<TestTransport>, Vec<_>) = (1..=2)
        .map(|mac| {
            let (inbound, rx) = mpsc::channel(4);
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
        .map(|(link, network)| RouterPort::new(link, network))
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

    let db = Arc::new(RwLock::new(database(vec![
        destination(address_recipient(1, &[0xFF]), 1, true),
        destination(address_recipient(9, &[0xFF]), 2, true),
        destination(address_recipient(9, &[0xFF]), 3, false),
    ])));
    let network = Arc::new(NetworkLayer::new(server));
    let comm_state = Arc::new(CommState::default());
    let suppressions: Arc<super::super::event_suppression::EventSuppressions> = Arc::default();
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));
    let notifications = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let device = fixture_device(&db).await;
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
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Network 1 carries the one unconfirmed copy, to the router at MAC 7;
    // network 2 carries nothing.
    let sent = apdus(&logs[0]);
    let [copy] = sent.as_slice() else {
        panic!("network 1: one APDU, not {sent:?}")
    };
    assert_eq!(copy.mac.as_slice(), [7]);
    let to = copy.decode_npdu().destination.unwrap();
    assert_eq!((to.network, to.mac_address.as_slice()), (9, &[0xFF][..]));
    assert!(matches!(
        copy.apdu(),
        Apdu::UnconfirmedRequest(request)
            if request.service_choice
                == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
    ));
    assert!(apdus(&logs[1]).is_empty(), "nothing on network 2");
    assert_eq!(suppressions.snapshot(), counted(2));
    assert_eq!(notifications.active_count(), 0, "no confirmed worker");
    router.stop().await.unwrap();
}
