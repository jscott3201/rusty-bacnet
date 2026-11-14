//! Handles from each source, pinned and following, and the address as the
//! key (#2560), against real servers on an in-memory link (`test_bus`) and,
//! for routed stations, behind an in-process router.

use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::test_bus::{Bus, at, requests, server};
use super::*;
use crate::client::{Client, WhoIsScope, unreachable};
use crate::format::decode_all;
use crate::limits::RequestLimits;

/// The instance the tests' devices share.
const N: u32 = 1001;

fn ai() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// A client at station 1 of `bus`, timing out in one second, and its sends.
async fn client(bus: &Arc<Bus>) -> (Client<TestTransport>, SendLog) {
    let port = bus.port(1);
    let sent = port.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let low = BACnetClient::start(config, port).await.unwrap();
    let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
    (client, sent)
}

/// The analog input's name, as `device` reads it.
async fn name_of<T: TransportPort + 'static>(device: &Device<T>) -> Result<String, Error> {
    let data = device
        .read_property(ai(), PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    match decode_all(&data).as_deref() {
        Some([PropertyValue::CharacterString(name)]) => Ok(name.clone()),
        other => panic!("a name: {other:?}"),
    }
}

/// Let the link's frames land.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn stop(servers: Vec<BACnetServer<TestTransport>>) {
    for mut server in servers {
        server.stop().await.unwrap();
    }
}

/// From an address a handle is pinned, from an instance claimed once it
/// follows, from a discovery result it is pinned and knows the instance;
/// making any of them sends nothing, and each reads the device.
#[tokio::test(start_paused = true)]
async fn handles_from_each_source_send_nothing() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let found = client
        .discover(WhoIsScope::Local, None, Duration::from_millis(500))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].address.clone(), found[0].instance), (at(2), N));
    assert!(!found[0].duplicate);
    let before = sent.len();
    let by_text = client.device("10.0.0.2:47808").await.unwrap();
    let by_address = client.device(at(2)).await.unwrap();
    let by_instance = client.device(N).await.unwrap();
    let by_instance_text = client.device("1001").await.unwrap();
    let by_found = client.device(&found[0]).await.unwrap();
    assert_eq!(sent.len(), before, "making handles sent nothing");
    for (handle, follows) in [
        (&by_text, false),
        (&by_address, false),
        (&by_instance, true),
        (&by_instance_text, true),
        (&by_found, false),
    ] {
        assert_eq!(handle.follows(), follows, "{handle:?}");
        assert_eq!((handle.address(), handle.instance()), (at(2), Some(N)));
        assert_eq!(name_of(handle).await.unwrap(), "A");
    }
    stop(vec![lab]).await;
}

/// A handle from an address the table doesn't hold reads the instance by
/// the wildcard once, and the client remembers it for the address.
#[tokio::test(start_paused = true)]
async fn the_wildcard_read_happens_once() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let device = client.device(at(2)).await.unwrap();
    assert_eq!(device.instance(), None);
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    assert_eq!(device.describe_device().await.unwrap().device, N);
    assert_eq!(requests(&sent, rp, 2), 1, "the wildcard read");
    assert_eq!(device.describe_device().await.unwrap().device, N);
    let again = client.device(at(2)).await.unwrap();
    assert_eq!(again.instance(), Some(N), "remembered for the address");
    assert_eq!(again.resolve_instance().await.unwrap(), N);
    assert_eq!(requests(&sent, rp, 2), 1, "read once");
    stop(vec![lab]).await;
}

/// Two devices sharing an instance: each is reached by its address, and
/// the instance alone is refused, naming both.
#[tokio::test(start_paused = true)]
async fn a_shared_instance_is_reached_by_address() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    let found = client
        .discover(WhoIsScope::Local, None, Duration::from_millis(500))
        .await
        .unwrap();
    assert!(found.iter().all(|found| found.duplicate), "{found:?}");
    let duplicates = client.duplicates().await;
    assert_eq!(
        duplicates,
        [Duplicate {
            instance: N,
            addresses: vec![at(2), at(3)],
        }]
    );
    let error = client.device(N).await.unwrap_err();
    assert!(matches!(error, Error::AmbiguousDevice { instance: N, .. }));
    let text = error.to_string();
    assert!(
        text.contains("10.0.0.2:47808") && text.contains("10.0.0.3:47808"),
        "{text}"
    );
    let a = client.device("10.0.0.2:47808").await.unwrap();
    let b = client.device(at(3)).await.unwrap();
    assert_eq!(name_of(&a).await.unwrap(), "A");
    assert_eq!(name_of(&b).await.unwrap(), "B");
    stop(servers).await;
}

/// A following handle whose device stops answering moves to the one other
/// address claiming its instance, says so, and reads there.
#[tokio::test(start_paused = true)]
async fn a_following_handle_moves_to_the_one_other_claim() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let old = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    let new = server(bus.port(3), N, "B").await;
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    let mut events = client.events();
    assert_eq!(name_of(&device).await.unwrap(), "B");
    assert_eq!(device.address(), at(3));
    assert_eq!(
        events.try_recv().unwrap(),
        ClientEvent::Moved {
            instance: N,
            from: at(2),
            to: at(3),
        }
    );
    stop(vec![old, new]).await;
}

/// Two other addresses claiming the instance leave a following handle where
/// it was, with the ambiguity as its error, and nothing moved.
#[tokio::test(start_paused = true)]
async fn a_following_handle_stays_when_two_others_claim_it() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
        server(bus.port(4), N, "C").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    bus.i_am(3, N);
    bus.i_am(4, N);
    settle().await;
    bus.silence(2);
    let mut events = client.events();
    let error = name_of(&device).await.unwrap_err();
    assert!(
        matches!(error, Error::AmbiguousDevice { instance: N, .. }),
        "{error}"
    );
    assert_eq!(device.address(), at(2));
    assert!(events.try_recv().is_err(), "nothing moved");
    stop(servers).await;
}

/// A pinned handle never follows: its read times out at its address, and
/// `moved_to` names the other address once its own has been quiet long
/// enough; two devices announcing together are no move.
#[tokio::test(start_paused = true)]
async fn a_pinned_handle_reports_a_move_and_stays() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    assert_eq!(device.instance(), Some(N));
    assert_eq!(device.moved_to().await, None, "two devices, one Who-Is");
    tokio::time::sleep(QUIET * 2).await;
    bus.i_am(3, N);
    settle().await;
    assert_eq!(device.moved_to().await, Some(at(3)));
    bus.silence(2);
    let error = name_of(&device).await.unwrap_err();
    assert!(unreachable(&error), "{error}");
    assert_eq!(device.address(), at(2), "never followed");
    stop(servers).await;
}

/// Another device answering at the old address, with no I-Am, keeps the
/// row's age young but not its announcement: the move still shows.
#[tokio::test(start_paused = true)]
async fn a_reused_address_that_answers_still_shows_the_move() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    // Device 2002 took station 2, which last announced N.
    let servers = vec![
        server(bus.port(2), 2002, "other").await,
        server(bus.port(3), N, "B").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    assert_eq!(device.instance(), Some(N));
    tokio::time::sleep(QUIET * 2).await;
    bus.i_am(3, N);
    settle().await;
    assert_eq!(
        name_of(&device).await.unwrap(),
        "other",
        "station 2 answers"
    );
    assert_eq!(device.moved_to().await, Some(at(3)));
    stop(servers).await;
}

/// A device named by its instance and by its address waits at one gate.
#[tokio::test(start_paused = true)]
async fn one_gate_per_address_however_named() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let held = client.limits().gate(&at(2)).acquire_owned().await.unwrap();
    let wait = Duration::from_secs(30);
    for device in [
        client.device(N).await.unwrap(),
        client.device("10.0.0.2:47808").await.unwrap(),
    ] {
        let read = tokio::time::timeout(wait, name_of(&device)).await;
        assert!(read.is_err(), "{device:?} went past the gate");
    }
    drop(held);
    assert_eq!(
        name_of(&client.device(N).await.unwrap()).await.unwrap(),
        "A"
    );
    stop(vec![lab]).await;
}

/// No address claims the instance: one Who-Is finds the device, or the
/// handle fails as unknown, sending nothing when the client may not ask.
#[tokio::test(start_paused = true)]
async fn an_unclaimed_instance_is_asked_for_once() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let device = client.device(N).await.unwrap();
    assert_eq!(device.address(), at(2));
    assert_eq!(sent.broadcasts().len(), 1, "one Who-Is");
    let error = client.device(4242).await.unwrap_err();
    assert!(matches!(&error, Error::UnknownDevice { device } if device == "4242"));
    client.set_resolve_wait(None);
    let before = sent.len();
    assert!(client.device(4243).await.is_err());
    assert_eq!(sent.len(), before, "no Who-Is");
    stop(vec![lab]).await;
}
