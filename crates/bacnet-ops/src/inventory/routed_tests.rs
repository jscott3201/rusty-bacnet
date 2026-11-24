//! An inventory of a device behind a router whose route the client loses
//! part way (#2620): the records read before it are kept, the rest are
//! `unread`, `stopped` says why, and nothing more is sent.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, SendMode, SentFrame, StartMode, TestTransport};
use bacnet_types::enums::ObjectType;
use tokio::sync::Notify;

use super::InventoryOptions;
use super::test_lab::{LAB, objects, oid};
use crate::client::Discover;
use crate::device::DeviceAddress;
use crate::device::test_bus::{Bus, client_on, mac};

/// The confirmed requests in `log`, in the order sent: the network each
/// went to, and its service (12 ReadProperty, 14 ReadPropertyMultiple).
fn requests(log: &SendLog) -> Vec<(u16, u8)> {
    log.frames().iter().filter_map(confirmed).collect()
}

/// `frame`'s destination network and service, when it is a confirmed
/// request.
fn confirmed(frame: &SentFrame) -> Option<(u16, u8)> {
    let npdu = frame.decode_npdu();
    if npdu.is_network_message() {
        return None;
    }
    match decode_apdu(npdu.payload).ok()? {
        Apdu::ConfirmedRequest(request) => {
            let network = npdu.destination.map_or(0, |to| to.network);
            Some((network, request.service_choice.to_raw()))
        }
        _ => None,
    }
}

/// Holds the client's `at`th confirmed request until the test says go.
#[derive(Default)]
struct Hold {
    count: AtomicUsize,
    /// The held request is waiting.
    held: Notify,
    /// Let it go.
    go: Notify,
}

/// The lab device at station 5 of network 2, behind a router at station
/// 254 of networks 1 and 2, and a client at station 1 of network 1 that
/// has found it there. The client's third confirmed request, the first
/// batch's, waits in its link while the test drops the client's route to
/// network 2 (its discovery table row); the batch then goes on, and the
/// second batch has no route. The first nine objects are records, the
/// other four are unread, and no request follows the first batch.
#[tokio::test(start_paused = true)]
async fn a_route_lost_part_way_keeps_what_was_read() {
    let (near, two) = (Bus::new(), Bus::new());
    let ports = vec![
        RouterPort::new(near.port(254), 1),
        RouterPort::new(two.port(254), 2),
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let identity = DeviceIdentity::new(LAB, 42).unwrap().with_name("Lab");
    let database = build_database_with_extra(&identity, objects()).unwrap();
    let mut server = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(two.port(5))
        .build()
        .await
        .unwrap();

    let hold = Arc::new(Hold::default());
    let port = {
        let hold = Arc::clone(&hold);
        TestTransport::builder()
            .inner(near.port(1))
            .unicast(SendMode::Forward)
            .broadcast(SendMode::Forward)
            .start(StartMode::Closed)
            .on_send(move |frame| {
                let hold = Arc::clone(&hold);
                let third = confirmed(&frame).is_some()
                    && hold.count.fetch_add(1, Ordering::Relaxed) + 1 == 3;
                async move {
                    if third {
                        hold.held.notify_one();
                        hold.go.notified().await;
                    }
                    Ok(())
                }
            })
            .build()
    };
    let (client, log) = client_on(port).await;
    let found = client
        .discover(Discover::network(2).wait(Duration::from_millis(500)))
        .await
        .unwrap();
    let station = DeviceAddress::new(2, &mac(5)).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].address, station);
    let device = client.device(station.clone()).await.unwrap();

    let options = InventoryOptions::new();
    let (read, ()) = tokio::join!(device.inventory(&options), async {
        hold.held.notified().await;
        client.low_level().clear_devices().await;
        hold.go.notify_one();
    });
    let inventory = read.unwrap();
    // In Object_List order: the Device object, then each as built.
    let built = objects()
        .into_iter()
        .map(|object| object.object_identifier());
    let objects: Vec<_> = std::iter::once(oid(ObjectType::DEVICE, LAB))
        .chain(built)
        .collect();
    let read: Vec<_> = inventory.points.iter().map(|point| point.object).collect();
    assert_eq!(read, objects[..9], "the first batch's records");
    assert!(inventory.points.iter().all(|point| point.device == station));
    assert_eq!(inventory.unread, objects[9..]);
    let stopped = inventory.stopped.as_ref().expect("a reason");
    assert!(!stopped.timed_out(), "nothing waited: {stopped}");
    assert_eq!(
        requests(&log),
        [(2, 14), (2, 12), (2, 14)],
        "the summary, the Object_List and the first batch, routed; none after"
    );

    server.stop().await.unwrap();
    router.stop().await.unwrap();
}
