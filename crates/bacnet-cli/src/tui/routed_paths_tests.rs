//! `:path-limit` and the browser's routed path hint (#2667), against a
//! client whose router refuses every request as too long; the path named
//! by a device of the table (#2726).

use std::time::Duration;

use bacnet_client::client::{ClientConfig, Station};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ConfirmedServiceChoice, Segmentation};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::*;

const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
const DNET: u16 = 2;
const STATION: u8 = 5;

/// A reason-4 (too long) Reject-Message-To-Network for `DNET` from the
/// router.
fn too_long() -> ReceivedNpdu {
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK),
            payload: Bytes::copy_from_slice(&[
                RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
                (DNET >> 8) as u8,
                DNET as u8,
            ]),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu::unverified(buf.freeze(), ROUTER.into(), false, Vec::new(), None)
}

/// A client whose router answers every unicast with a too-long reject,
/// with station 5 on network 2 behind it in the table.
async fn site() -> BACnetClient<TestTransport> {
    let (inbox, inbound) = mpsc::channel(32);
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(inbound)
        .on_send(move |frame| {
            let inbox = inbox.clone();
            async move {
                if !frame.broadcast {
                    let _ = inbox.send(too_long()).await;
                }
                Ok(())
            }
        })
        .build();
    let config = ClientConfig {
        apdu_timeout_ms: 500,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    client
        .add_routed_device(bacnet_client::discovery::RoutedDeviceConfig {
            instance: 77,
            router_mac: ROUTER.to_vec(),
            remote_network: DNET,
            remote_mac: vec![STATION],
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    client
}

async fn hint_of(
    client: &BACnetClient<TestTransport>,
    address: &DeviceAddress,
    error: &Error,
) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(1), hint(client, 77, address, error))
        .await
        .unwrap()
}

/// A small read the router refuses as too long times out, since the client
/// doesn't believe the reject; the hint names the path and the command.
/// Setting the limit clears the path's mark, so the hint goes; a failure
/// other than a timeout, or a device on this network, never has one.
#[tokio::test(start_paused = true)]
async fn a_timeout_after_a_small_request_refused_hints_at_the_path_limit() {
    let mut client = site().await;
    let station = DeviceAddress::new(DNET, &[STATION]).unwrap();
    let error = client
        .confirmed_request(
            &Station::routed(&ROUTER, DNET, &[STATION]),
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x0c, 0x02, 0x00, 0x00, 0x4d, 0x19, 0x4d],
        )
        .await
        .unwrap_err();
    assert!(bacnet_ops::raw::timed_out(&error), "{error:?}");
    assert_eq!(client.below_minimum_reject_drops(), 1);

    let hint = hint_of(&client, &station, &error).await.expect("a hint");
    assert!(hint.contains("The router at 10.0.0.9:47808"), "{hint}");
    // The device's own form first, then the path's (#2726).
    assert!(
        hint.contains(":path-limit 2:77 SIZE (or :path-limit 10.0.0.9:47808/2=SIZE)"),
        "{hint}"
    );
    let other = Error::invalid_argument("no");
    assert_eq!(hint_of(&client, &station, &other).await, None);
    let local = DeviceAddress::local(&ROUTER).unwrap();
    assert_eq!(hint_of(&client, &local, &error).await, None);

    let set = PathLimit::parse(&["10.0.0.9:47808/2=128"]).unwrap();
    assert_eq!(
        set.apply(&client).await,
        "Requests to network 2 through 10.0.0.9:47808 now go in at most 128 NPDU octets"
    );
    assert_eq!(
        client.routed_path_limits()[0].configured_max_npdu,
        Some(128)
    );
    assert_eq!(hint_of(&client, &station, &error).await, None);

    let clear = PathLimit::parse(&["10.0.0.9:47808/2", "clear"]).unwrap();
    assert!(
        clear
            .apply(&client)
            .await
            .contains("back on the default of 228")
    );
    assert!(client.routed_path_limits().is_empty());
    client.stop().await.unwrap();
}

/// The browse worker sends the hint ahead of the failure it explains: a
/// read of the station behind the router that refuses everything times out,
/// and the worker delivers the hint, then the failure, for the same request.
/// The browser relies on that order for an object read, whose request it
/// forgets once the read has failed, so both kinds of read are checked.
#[tokio::test(start_paused = true)]
async fn the_browse_worker_sends_the_hint_before_the_failure() {
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use crate::tui::point::BrowseRead;

    hint_then_failure(BrowseRead::Device).await;
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    hint_then_failure(BrowseRead::Object(ai1)).await;
}

async fn hint_then_failure(read: crate::tui::point::BrowseRead) {
    use crate::tui::message::{Command, WorkerEvent};
    use crate::tui::point::BrowseResult;

    let mut client = site().await;
    let rate = bacnet_ops::limits::RequestLimits::new(0);
    let (sink, mut events) = crate::tui::worker::EventSink::channel(16);
    let (commands, commands_rx) = mpsc::channel(4);
    commands
        .send(Command::Browse {
            req: 7,
            device: 77,
            at: DeviceAddress::new(DNET, &[STATION]).unwrap(),
            read: read.clone(),
        })
        .await
        .unwrap();
    let mut results = Vec::new();
    {
        let worker = crate::tui::worker::browse::run(&client, &sink, commands_rx, &rate);
        tokio::pin!(worker);
        // Up to the failure, which ends the read.
        while !matches!(results.last(), Some((_, BrowseResult::Failed(_)))) {
            tokio::select! {
                () = &mut worker => panic!("the browse worker ended"),
                event = events.recv() => match event.unwrap() {
                    WorkerEvent::Browse { req, result } => results.push((req, result)),
                    other => panic!("unexpected {other:?}"),
                },
            }
        }
    }
    let [
        (7, BrowseResult::PathHint(hint)),
        (7, BrowseResult::Failed(error)),
    ] = results.as_slice()
    else {
        panic!("expected a hint, then a failure, for {read:?}: {results:?}");
    };
    assert!(hint.contains(":path-limit 10.0.0.9:47808/2=SIZE"), "{hint}");
    assert!(!error.is_empty());
    drop(commands);
    client.stop().await.unwrap();
}

/// The command refuses what the client refuses, naming the path: a limit
/// too small for any request, and a router given as a device.
#[tokio::test(start_paused = true)]
async fn the_command_reports_what_it_refused() {
    let mut client = site().await;
    let tiny = PathLimit::parse(&["10.0.0.9:47808/2=10"]).unwrap();
    let said = tiny.apply(&client).await;
    assert!(
        said.starts_with("path-limit 10.0.0.9:47808/2: a routed path limit of 10"),
        "{said}"
    );
    let device = PathLimit::parse(&["1234/2=480"]).unwrap();
    assert!(device.apply(&client).await.contains("not a device"));
    assert!(client.routed_path_limits().is_empty());
    client.stop().await.unwrap();
}

#[test]
fn the_command_takes_a_limit_or_clear() {
    assert_eq!(
        PathLimit::parse(&["192.168.1.20:47808/5=128"]),
        Ok(PathLimit {
            path: PathTo::Router {
                router: "192.168.1.20:47808".into(),
                network: 5,
            },
            max_npdu: Some(128),
        })
    );
    assert_eq!(
        PathLimit::parse(&["mstp:3/7", "clear"]),
        Ok(PathLimit {
            path: PathTo::Router {
                router: "mstp:3".into(),
                network: 7,
            },
            max_npdu: None,
        })
    );
    for words in [
        &[][..],
        &["192.168.1.20:47808/5"][..],
        &["192.168.1.20:47808/0=128"][..],
        &["192.168.1.20:47808/5", "reset"][..],
        &["192.168.1.20:47808/5=128", "clear"][..],
    ] {
        assert!(PathLimit::parse(words).is_err(), "{words:?}");
    }
    // Each refusal names what was typed.
    assert_eq!(
        PathLimit::parse(&["foo", "clear"]),
        Err("'foo' is not ROUTER/NETWORK".into())
    );
    assert_eq!(
        PathLimit::parse(&["/5", "clear"]),
        Err("'/5' names no router".into())
    );
    // A size as a word of its own belongs to the device form.
    assert_eq!(
        PathLimit::parse(&["10.0.0.9:47808/2", "480"]),
        Err("'10.0.0.9:47808/2 480': a path takes its size after '=': \
             path-limit 10.0.0.9:47808/2=480"
            .into())
    );
    // A word after a whole ROUTER/NETWORK=SIZE is one too many.
    assert_eq!(
        PathLimit::parse(&["10.0.0.9:47808/2=128", "5"]),
        Err(USAGE.into())
    );
    assert_eq!(PathLimit::size("clear"), Ok(None));
    assert_eq!(PathLimit::size("480"), Ok(Some(480)));
    assert!(
        PathLimit::size("reset")
            .unwrap_err()
            .contains("neither clear nor")
    );
}

/// A device of the table names the path the client reaches it by (#2726):
/// the router and network its discovery row holds, the same path the
/// router form names. A device on this network has no path, and a device
/// the client no longer knows is refused; neither changes anything.
#[tokio::test(start_paused = true)]
async fn a_device_names_the_path_it_is_reached_by() {
    let mut client = site().await;
    let device = |instance, at| PathLimit {
        path: PathTo::Device { instance, at },
        max_npdu: Some(128),
    };
    let station = DeviceAddress::new(DNET, &[STATION]).unwrap();
    assert_eq!(
        device(77, station.clone()).apply(&client).await,
        "Requests to network 2 through 10.0.0.9:47808 (device 77's path) now go in at most \
         128 NPDU octets"
    );
    let limits = client.routed_path_limits();
    assert_eq!(
        (limits[0].router_mac.as_slice(), limits[0].dnet),
        (&ROUTER[..], DNET)
    );
    assert_eq!(limits[0].configured_max_npdu, Some(128));

    let local = DeviceAddress::local(&[10, 0, 0, 5, 0xBA, 0xC0]).unwrap();
    assert_eq!(
        device(9, local).apply(&client).await,
        "path-limit device 9: device 9 at 10.0.0.5:47808 is on this network, not behind a \
         router, so no routed path's limit applies to it"
    );
    let unknown = DeviceAddress::new(3, &[7]).unwrap();
    let said = device(8, unknown).apply(&client).await;
    assert!(
        said.starts_with("path-limit device 8: device 3:0x07"),
        "{said}"
    );
    assert_eq!(
        client.routed_path_limits()[0].configured_max_npdu,
        Some(128)
    );

    let clear = PathLimit {
        max_npdu: None,
        ..device(77, station)
    };
    assert_eq!(
        clear.apply(&client).await,
        "Requests to network 2 through 10.0.0.9:47808 (device 77's path) are back on the \
         default of 228 NPDU octets"
    );
    assert!(client.routed_path_limits().is_empty());
    client.stop().await.unwrap();
}
