//! Learning the network's number folds the discovery table (#2572): routed
//! rows on that network name local stations. A row folds into a local row
//! that holds what a device announced, and takes the place of one that
//! holds only `add_device`'s placeholders. A changed number drops the rows
//! that were local only by the old one.
use super::asking::open_site;
use super::local_routes::{
    PEER, REMOTE_NETWORK, ROUTER, THIS_NETWORK, i_am, relayed_from, send_to_client,
};
use crate::client::{DeviceEvent, DeviceEventKind};
use bacnet_types::error::Error;
use tokio::sync::broadcast;
use tokio::time::Duration;

/// Take the next notice, checked to be `kind` for Device `instance` at
/// `mac`, routed on `network` or local for `None`.
fn expect(
    events: &mut broadcast::Receiver<DeviceEvent>,
    kind: DeviceEventKind,
    instance: u32,
    network: Option<u16>,
    mac: &[u8],
) -> DeviceEvent {
    let event = events.try_recv().expect("a device notice");
    assert_eq!(event.kind, kind, "{event:?}");
    let device = &event.device;
    assert_eq!(device.object_identifier.instance_number(), instance);
    assert_eq!(device.source_network, network, "{device:?}");
    let station = device
        .source_address
        .as_ref()
        .unwrap_or(&device.mac_address);
    assert_eq!(station.as_slice(), mac);
    event
}

/// Learning the number folds every routed row on this network in one
/// locked call: one whose station has a local row is taken out, one with
/// no local row becomes it, keeping what its I-Am said. Each is reported
/// `Lost`, the row it became `Discovered`, then one collision notice per
/// instance whose claimants changed. A row on another network stays.
#[tokio::test(start_paused = true)]
async fn learning_the_number_folds_the_routed_rows_on_this_network() {
    let mut site = open_site(true).await;
    let relayed = |network, station: &[u8]| relayed_from(network, station);
    let inbound = &site.inbound;
    // Device 9 at PEER, also heard relayed back; device 10 only relayed;
    // device 11 on another network; device 12 relayed from both networks.
    send_to_client(inbound, &i_am(9), &PEER, None).await;
    send_to_client(inbound, &i_am(9), &ROUTER, relayed(THIS_NETWORK, &PEER)).await;
    send_to_client(inbound, &i_am(10), &ROUTER, relayed(THIS_NETWORK, &[4])).await;
    send_to_client(inbound, &i_am(11), &ROUTER, relayed(REMOTE_NETWORK, &[4])).await;
    send_to_client(inbound, &i_am(12), &ROUTER, relayed(THIS_NETWORK, &[6])).await;
    send_to_client(inbound, &i_am(12), &ROUTER, relayed(REMOTE_NETWORK, &[6])).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(site.client.discovered_devices().await.len(), 6);

    let mut events = site.client.device_events();
    let mut collisions = site.client.device_collision_events();
    site.announce(THIS_NETWORK, 0).await;
    assert_eq!(site.client.discovered_devices().await.len(), 5);

    let ev = &mut events;
    expect(ev, DeviceEventKind::Lost, 9, Some(THIS_NETWORK), &PEER);
    expect(ev, DeviceEventKind::Lost, 10, Some(THIS_NETWORK), &[4]);
    expect(ev, DeviceEventKind::Discovered, 10, None, &[4]);
    expect(ev, DeviceEventKind::Lost, 12, Some(THIS_NETWORK), &[6]);
    expect(ev, DeviceEventKind::Discovered, 12, None, &[6]);
    assert!(events.try_recv().is_err(), "nothing else changed");

    // Device 9's contest ended; device 12's goes on, with its local row.
    let ended = collisions.try_recv().unwrap();
    assert_eq!((ended.instance, ended.claimants.len()), (9, 1));
    let goes_on = collisions.try_recv().unwrap();
    assert_eq!(goes_on.instance, 12);
    let networks: Vec<_> = goes_on
        .claimants
        .iter()
        .map(|claimant| claimant.source_network)
        .collect();
    assert_eq!(networks, [None, Some(REMOTE_NETWORK)]);
    assert!(collisions.try_recv().is_err());

    let device = site.client.get_device(10).await.unwrap().unwrap();
    assert!(device.is_local());
    assert_eq!(device.mac_address.as_slice(), [4]);
    assert_eq!((device.max_apdu_length, device.vendor_id), (480, 260));
    assert!(device.last_announced.is_some(), "its I-Am's age kept");
    let device = site.client.get_device(11).await.unwrap().unwrap();
    assert_eq!(device.source_network, Some(REMOTE_NETWORK));
    assert!(site.client.get_device(9).await.unwrap().unwrap().is_local());
    assert!(matches!(
        site.client.get_device(12).await,
        Err(Error::AmbiguousDevice { instance: 12, .. })
    ));
    site.client.stop().await.unwrap();
}

/// A station registered with `add_device` holds placeholders (1,476
/// octets, no segmentation, vendor 0), not what the device says. When a
/// routed row from its relayed I-Am folds in, the device's own figures
/// take their place: `Updated` for the same instance, and for another
/// instance the placeholder `Lost` and the device `Discovered`.
#[tokio::test(start_paused = true)]
async fn a_fold_keeps_what_the_device_announced_over_a_placeholder() {
    let mut site = open_site(true).await;
    site.client.add_device(9, &PEER).await.unwrap();
    site.client.add_device(8, &[5]).await.unwrap();
    let relayed = |station: &[u8]| relayed_from(THIS_NETWORK, station);
    send_to_client(&site.inbound, &i_am(9), &ROUTER, relayed(&PEER)).await;
    send_to_client(&site.inbound, &i_am(10), &ROUTER, relayed(&[5])).await;
    tokio::time::sleep(Duration::from_millis(5)).await;

    let mut events = site.client.device_events();
    site.announce(THIS_NETWORK, 0).await;
    let ev = &mut events;
    expect(ev, DeviceEventKind::Lost, 9, Some(THIS_NETWORK), &PEER);
    let updated = expect(ev, DeviceEventKind::Updated, 9, None, &PEER);
    assert_eq!(updated.device.max_apdu_length, 480);
    expect(ev, DeviceEventKind::Lost, 10, Some(THIS_NETWORK), &[5]);
    expect(ev, DeviceEventKind::Lost, 8, None, &[5]);
    expect(ev, DeviceEventKind::Discovered, 10, None, &[5]);
    assert!(events.try_recv().is_err());

    let devices = site.client.discovered_devices().await;
    assert_eq!(devices.len(), 2, "{devices:?}");
    for (instance, mac) in [(9, PEER), (10, [5])] {
        let device = site.client.get_device_at(0, &mac).await.unwrap();
        assert_eq!(device.object_identifier.instance_number(), instance);
        assert_eq!(
            (device.max_apdu_length, device.vendor_id),
            (480, 260),
            "what the I-Am said, not the placeholder"
        );
    }
    assert_eq!(
        site.client
            .get_device(8)
            .await
            .unwrap()
            .map(|d| d.vendor_id),
        None
    );
    site.client.stop().await.unwrap();
}

/// A learned number replaced by a configured one (a router's
/// announcement): the rows local only by the old number, folded from a
/// routed row or a relay landed there, are dropped and reported `Lost`; a
/// device heard directly stays, and so does a folded row its device's own
/// I-Am, with no SNET, has refreshed since; routed rows on the new number
/// fold.
#[tokio::test(start_paused = true)]
async fn a_changed_number_drops_the_rows_local_by_the_old_one() {
    const NEW_NETWORK: u16 = THIS_NETWORK + 1;
    let mut site = open_site(true).await;
    let relayed = |network, station: &[u8]| relayed_from(network, station);
    let inbound = &site.inbound;
    send_to_client(inbound, &i_am(9), &PEER, None).await;
    send_to_client(inbound, &i_am(9), &ROUTER, relayed(THIS_NETWORK, &PEER)).await;
    send_to_client(inbound, &i_am(10), &ROUTER, relayed(THIS_NETWORK, &[4])).await;
    send_to_client(inbound, &i_am(14), &ROUTER, relayed(THIS_NETWORK, &[8])).await;
    send_to_client(inbound, &i_am(13), &ROUTER, relayed(NEW_NETWORK, &[7])).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    site.announce(THIS_NETWORK, 0).await;
    // A relay after learning lands on a local row by the number alone;
    // device 14, folded like device 10, is then heard directly.
    send_to_client(inbound, &i_am(12), &ROUTER, relayed(THIS_NETWORK, &[6])).await;
    send_to_client(inbound, &i_am(14), &[8], None).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(site.client.get_device_at(0, &[6]).await.is_some());

    let mut events = site.client.device_events();
    site.announce(NEW_NETWORK, 1).await;
    let ev = &mut events;
    expect(ev, DeviceEventKind::Lost, 10, None, &[4]);
    expect(ev, DeviceEventKind::Lost, 12, None, &[6]);
    expect(ev, DeviceEventKind::Lost, 13, Some(NEW_NETWORK), &[7]);
    expect(ev, DeviceEventKind::Discovered, 13, None, &[7]);
    assert!(events.try_recv().is_err());

    let devices = site.client.discovered_devices().await;
    let local: Vec<_> = devices
        .iter()
        .map(|d| (d.object_identifier.instance_number(), d.is_local()))
        .collect();
    assert_eq!(local, [(9, true), (13, true), (14, true)], "{devices:?}");
    site.client.stop().await.unwrap();
}
