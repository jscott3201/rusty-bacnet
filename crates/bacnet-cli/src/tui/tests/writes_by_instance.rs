//! A `:writem` batch whose device the command bar named by its instance is
//! placed where the device answers as it before anything is read (#2824):
//! the table's one claim is only a cue, as anyone can announce an
//! instance. The lab device is 100 at 10.0.0.100; a second station at
//! 10.0.0.101 answers as 100 too. On paused time.

use bacnet_ops::DeviceAddress;

use super::site::{ClientLimits, Server, Site};
use super::writes::*;
use super::*;
use crate::tui::profile::Safety;
use crate::tui::safety::{SafetyCommand, SafetyEvent};

const IMPOSTOR_MAC: [u8; 6] = [10, 0, 0, 101, 0xBA, 0xC0];

/// The batch for `specs` named by instance 100, as the command bar sends it
/// once the table's one claim gave the specs their address.
fn by_instance(req: u64, address: DeviceAddress) -> Command {
    let mut spec = spec(Some("42"), 12);
    spec.address = address;
    Command::Safety(SafetyCommand::Prepare {
        req,
        specs: vec![spec],
        batch: true,
        named: Some((LAB, None)),
    })
}

/// The table's claim is the impostor's address: with the impostor
/// answering as 100 too, the Who-Is draws the lab device's claim and the
/// batch is refused, naming both, with nothing read or written at either.
/// With the impostor silent, the batch is placed at the lab device: its
/// preview is read there, and nothing goes to the impostor but the
/// question.
#[tokio::test(start_paused = true)]
async fn a_batch_by_instance_goes_where_the_device_answers() {
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![Server::lab(100, LAB), Server::bare(101, LAB)];
    let options = options(dir.path(), Safety::default(), false);
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    arm(&mut site).await;
    let impostor = DeviceAddress::local(&IMPOSTOR_MAC).unwrap();
    site.send(vec![by_instance(9, impostor.clone())]).await;
    let mut refused = None;
    site.pump_events_until(|event| match event {
        WorkerEvent::Safety(SafetyEvent::Refused {
            req: Some(9),
            reason,
        }) => {
            refused = Some(reason.clone());
            true
        }
        _ => false,
    })
    .await;
    let refused = refused.unwrap();
    assert!(
        refused.contains("10.0.0.100:47808") && refused.contains("10.0.0.101:47808"),
        "{refused}"
    );
    assert_eq!(writes_sent(&site), 0);
    let read_property = bacnet_types::enums::ConfirmedServiceChoice::READ_PROPERTY.to_raw();
    assert!(
        site.hub
            .services_to(&IMPOSTOR_MAC)
            .iter()
            .all(|service| *service == read_property),
        "the question alone at the impostor"
    );

    site.hub.silence(&IMPOSTOR_MAC);
    site.send(vec![by_instance(10, impostor)]).await;
    let mut placed = None;
    site.pump_events_until(|event| match event {
        WorkerEvent::Safety(SafetyEvent::Preview { req: 10, preview }) => {
            placed = Some(preview.items[0].spec.address.clone());
            true
        }
        WorkerEvent::Safety(SafetyEvent::Refused {
            req: Some(10),
            reason,
        }) => {
            panic!("{reason}")
        }
        _ => false,
    })
    .await;
    assert_eq!(placed, Some(DeviceAddress::local(&LAB_MAC).unwrap()));
    assert_eq!(writes_sent(&site), 0, "previewed, not confirmed");
    site.stop().await;
}
