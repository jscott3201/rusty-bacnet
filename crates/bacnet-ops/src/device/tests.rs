//! Handles from each source, pinned against following, and the address as
//! the key (#2560), against real servers on an in-memory link
//! (`test_bus`). Following handles that move are `follow_tests`, routed
//! stations `routed_tests`.

use std::time::Duration;

use bacnet_transport::test_transport::TestTransport;
use bacnet_types::data_link::DataLink;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;

use super::test_bus::{Bus, N, at, client, client_on, name_of, requests, server, settle, stop};
use super::*;
use crate::client::{WhoIsScope, unreachable};

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

/// The instance read by the wildcard goes when the discovery table loses
/// a row at the address: another device may answer there now.
#[tokio::test(start_paused = true)]
async fn a_lost_row_forgets_the_wildcard_read() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let device = client.device(at(2)).await.unwrap();
    assert_eq!(device.resolve_instance().await.unwrap(), N);
    bus.i_am(2, N);
    settle().await;
    assert_eq!(device.instance(), Some(N), "a row found is no loss");
    // Station 2 now announces another instance: its row for N is lost.
    bus.i_am(2, 2002);
    settle().await;
    assert_eq!(device.instance(), None, "forgotten with its row");
    assert_eq!(client.device(at(2)).await.unwrap().instance(), Some(2002));
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
/// handle fails as unknown. A clone that may not ask sends nothing, and
/// the other clones still ask.
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
    let silent = client.with_resolve_wait(None);
    assert_eq!(silent.clone().resolve_wait(), None, "kept by its clones");
    let before = sent.len();
    assert!(silent.device(4243).await.is_err());
    assert!(silent.device("5:0x05").await.is_err());
    assert_eq!(sent.len(), before, "no Who-Is");
    assert_eq!(client.resolve_wait(), Some(crate::client::RESOLVE_WAIT));
    assert!(client.device(4244).await.is_err());
    assert_eq!(sent.len(), before + 1, "the other clone still asks");
    stop(vec![lab]).await;
}

/// Two devices answering the one Who-Is for an instance no address
/// claimed: the handle is refused as ambiguous, not made for whichever
/// answered first.
#[tokio::test(start_paused = true)]
async fn two_answers_to_the_one_who_is_are_ambiguous() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    let error = client.device(N).await.unwrap_err();
    assert!(
        matches!(&error, Error::AmbiguousDevice { instance: N, candidates } if candidates.len() == 2),
        "{error}"
    );
    stop(servers).await;
}

/// An address is checked against the link as a target written for it is:
/// a one-octet MAC is an MS/TP station, which this link can't reach.
#[tokio::test(start_paused = true)]
async fn an_address_the_link_cant_reach_is_refused() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let station = DeviceAddress::local(&[5]).unwrap();
    let error = client.device(station.clone()).await.unwrap_err();
    assert!(
        matches!(&error, Error::OutOfRange(why) if why.contains("mstp:5")),
        "{error}"
    );
    let port = TestTransport::builder()
        .local_mac(&[1])
        .data_link(DataLink::Mstp)
        .build();
    let (mstp, _) = client_on(port).await;
    assert_eq!(mstp.device(station).await.unwrap().address().mac(), [5]);
    assert!(mstp.device(at(2)).await.is_err(), "six octets on MS/TP");
    assert!(sent.is_empty());
}

/// A routed address the discovery table has no row for has no path: a
/// read fails before it takes the device's turn, a rate slot or a charge,
/// by ReadProperty, by ReadPropertyMultiple and in a caller's own turn.
#[tokio::test(start_paused = true)]
async fn a_read_with_no_path_takes_no_turn() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;

    use crate::client::{Halt, Reader, Wanted};
    use crate::limits::{Account, charged};

    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let cancel = AtomicBool::new(false);
    let mut reader = Reader {
        client: client.low_level(),
        device: DeviceAddress::new(5, &[5]).unwrap(),
        instance: None,
        rate: client.limits(),
        cancel: &cancel,
        rpm: true,
    };
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let no_path = |halt: Halt| matches!(halt, Halt::Device(Error::UnknownDevice { .. }));
    let account = Arc::new(Account::agent(None));
    charged(Arc::clone(&account), async {
        let rp = reader.rp(ai, PropertyIdentifier::PRESENT_VALUE, None).await;
        assert!(no_path(rp.unwrap_err()));
        let rpm = reader.properties(ai, Wanted::All).await;
        assert!(no_path(rpm.unwrap_err()));
        assert!(reader.turn().await.is_err_and(no_path));
    })
    .await;
    assert_eq!(client.limits().recent(), 0, "no rate slot taken");
    assert_eq!(account.requests(), 0, "nothing charged");
    assert!(sent.is_empty());
}
