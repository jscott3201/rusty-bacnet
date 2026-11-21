//! Where a handle made from an instance asks for its device (#2560): a bare
//! instance on this network alone unless the client's resolve scope says
//! otherwise, a target naming its network on that network, and a handle
//! that stops answering on the network where it was last heard, never on
//! every network. On the in-memory link (`test_bus`); the routed lab has
//! the routed re-ask.

use bacnet_services::who_is::WhoIsRequest;

use super::discover_tests::{remote, the_who_is, who_is_frames};
use super::test_bus::{Bus, N, at, client, name_of, server, settle, stop};
use crate::OpsError;
use crate::client::ResolveScope;

/// The instance range of the one Who-Is in `log`, which stays.
fn asked_range(log: &bacnet_transport::test_transport::SendLog) -> Option<(u32, u32)> {
    let frames = who_is_frames(log);
    let [frame] = frames.as_slice() else {
        panic!("one Who-Is, not {frames:?}");
    };
    let bacnet_encoding::apdu::Apdu::UnconfirmedRequest(request) = frame.apdu() else {
        panic!("an unconfirmed request");
    };
    let decoded = WhoIsRequest::decode(&request.service_request).unwrap();
    decoded.range.map(|range| (range.low(), range.high()))
}

/// A bare instance no address claims is asked for on this network alone:
/// one local broadcast for that one instance, nothing global. A device
/// there answers it; a miss says only this network was asked and how to
/// reach further.
#[tokio::test(start_paused = true)]
async fn a_bare_instance_is_asked_for_on_this_network() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    assert_eq!(client.resolve_scope(), ResolveScope::Local);
    assert_eq!(ResolveScope::default(), ResolveScope::Local);
    let lab = server(bus.port(2), N, "A").await;
    let device = client.device(N).await.unwrap();
    assert_eq!(device.address(), at(2));
    assert_eq!(asked_range(&sent), Some((N, N)));
    assert_eq!(the_who_is(&sent), (None, true));
    let error = client.device(4242).await.unwrap_err();
    assert!(
        matches!(
            &error,
            OpsError::UnknownDevice { target, asked: Some(crate::Scope::Local) }
                if target == "4242"
        ),
        "{error:?}"
    );
    let text = error.to_string();
    for says in ["this network", "5:4242", "resolve scope"] {
        assert!(text.contains(says), "{says}: {text}");
    }
    assert_eq!(the_who_is(&sent), (None, true));
    stop(vec![lab]).await;
}

/// A wider resolve scope, set on a clone, is the client's choice and goes
/// out as such: to one network, or to every network; the miss says where.
/// The other clones keep asking on this network alone.
#[tokio::test(start_paused = true)]
async fn a_wider_resolve_scope_is_honoured() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    for (scope, to, says) in [
        (ResolveScope::Network(5), remote(5), "on network 5"),
        (ResolveScope::Global, remote(0xFFFF), "on any network"),
    ] {
        let wider = client.with_resolve_scope(scope);
        assert_eq!(wider.clone().resolve_scope(), scope, "kept by its clones");
        let error = wider.device(4242).await.unwrap_err();
        assert!(
            matches!(&error, OpsError::UnknownDevice { asked: Some(asked), .. } if *asked == scope.into()),
            "{error:?}"
        );
        assert!(error.to_string().contains(says), "{error}");
        assert_eq!(the_who_is(&sent), (to, true), "{scope:?}");
    }
    assert!(client.device(4242).await.is_err());
    assert_eq!(the_who_is(&sent), (None, true), "this clone asks locally");
}

/// A target that names its network asks that network, whatever the
/// resolve scope says.
#[tokio::test(start_paused = true)]
async fn a_named_network_is_asked_whatever_the_resolve_scope() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    for scope in [ResolveScope::Local, ResolveScope::Global] {
        let error = client
            .with_resolve_scope(scope)
            .device("5:4242")
            .await
            .unwrap_err();
        assert!(
            matches!(
                &error,
                OpsError::UnknownDevice { target, asked: Some(crate::Scope::Network(5)) }
                    if target == "5:4242"
            ),
            "{error:?}"
        );
        assert_eq!(the_who_is(&sent), (remote(5), true), "{scope:?}");
    }
}

/// A following handle whose request times out asks for its device again
/// on the network where it was last heard: here this one, a local
/// broadcast, even on a clone that resolves on every network.
#[tokio::test(start_paused = true)]
async fn the_re_ask_goes_where_the_device_was_last_heard() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client
        .with_resolve_scope(ResolveScope::Global)
        .device(N)
        .await
        .unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    sent.clear();
    bus.silence(2);
    let error = name_of(&device).await.unwrap_err();
    assert!(error.timed_out(), "{error}");
    assert_eq!(asked_range(&sent), Some((N, N)));
    assert_eq!(the_who_is(&sent), (None, true));
    stop(vec![lab]).await;
}

/// A resolve scope of network 0 or 65535 is the scope that number means,
/// this network or every network, so a lookup goes out instead of failing
/// as text to fix.
#[tokio::test(start_paused = true)]
async fn network_0_and_65535_resolve_scopes_are_local_and_global() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    for (set, is, to) in [
        (ResolveScope::Network(0), ResolveScope::Local, None),
        (
            ResolveScope::Network(0xFFFF),
            ResolveScope::Global,
            remote(0xFFFF),
        ),
    ] {
        let scoped = client.with_resolve_scope(set);
        assert_eq!(scoped.resolve_scope(), is);
        let error = scoped.device(4242).await.unwrap_err();
        assert!(
            matches!(&error, OpsError::UnknownDevice { asked: Some(asked), .. } if *asked == is.into()),
            "{error:?}"
        );
        assert_eq!(the_who_is(&sent), (to, true), "{set:?}");
    }
}

/// A remote address the table holds no row for asks the station, then its
/// network once, and a miss says so: the network named, with no hint to
/// widen anything. With no router known, both go out on the local link
/// naming network 5; neither is global.
#[tokio::test(start_paused = true)]
async fn a_remote_address_miss_says_its_station_was_asked() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let error = client.device("5:0x05").await.unwrap_err();
    assert!(
        matches!(
            &error,
            OpsError::UnknownDevice { target, asked: Some(crate::Scope::Network(5)) }
                if target == "5:0x05"
        ),
        "{error:?}"
    );
    assert!(!error.to_string().contains("resolve scope"), "{error}");
    let frames = who_is_frames(&sent);
    let [directed, network] = frames.as_slice() else {
        panic!("two Who-Is, not {frames:?}");
    };
    let to = directed.decode_npdu().destination.unwrap();
    assert_eq!((to.network, &to.mac_address[..]), (5, &[5][..]));
    assert_eq!(network.decode_npdu().destination, remote(5));
    assert!(directed.broadcast && network.broadcast);
}
