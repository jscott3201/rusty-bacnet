//! A BBMD's own B/IP address with a wildcard bind, at start and restart (#937).

use std::path::PathBuf;

use super::bbmd_start::{select_wildcard_bbmd_ip, BdtSource};
use super::own_broadcast_tests::{
    assert_no_bvll, assert_own_forwarded, port_of, recv_bvll, udp, NPDU,
};
use super::*;
use crate::port_ownership::{lost_port, restart, ATTEMPTS};

const PORT: u16 = 47808;
/// Stand-ins for the host's LAN addresses; tests only inject them.
const LAN: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const OTHER_LAN: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 7);
const REMOTE_PEER: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 5);

fn row(ip: Ipv4Addr, port: u16) -> BdtEntry {
    BdtEntry {
        ip: ip.octets(),
        port,
        broadcast_mask: [255; 4],
    }
}

fn select(
    rows: &[BdtEntry],
    local: &[Ipv4Addr],
    route: Option<Ipv4Addr>,
) -> Result<Ipv4Addr, Error> {
    select_wildcard_bbmd_ip(rows, PORT, local, route, BdtSource::Configured)
}

/// A UDP port that was free a moment ago.
fn free_port() -> u16 {
    std::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Starts the transport `build` makes for a [`free_port`], for the tests whose
/// BDT must name the bound port before the first start. Another process can
/// take the port between the probe and the start's bind (#1032), so a start
/// that loses the port goes again with a fresh port and a rebuilt transport, a
/// bounded number of times; the last attempt's result is returned whatever it
/// is. Removes the persisted BDT, if `build` set one, once each start returns.
///
/// The retry only sees a bind that fails. On macOS a socket holding the port on
/// 127.0.0.1 shadows the transport's SO_REUSEADDR wildcard bind instead of
/// failing it, and neither this helper nor the transport can detect that (see
/// `port_ownership`); the port is then lost silently and the test fails on its
/// own assertions. The window is the few microseconds between the probe and the
/// bind.
/// Gives back the transport, its port and the start's result.
async fn start_on_free_port(
    build: impl Fn(u16) -> BipTransport,
) -> (
    BipTransport,
    u16,
    Result<mpsc::Receiver<ReceivedNpdu>, Error>,
) {
    let mut attempt = 1;
    loop {
        let port = free_port();
        let mut transport = build(port);
        let started = transport.start().await;
        if let Some(path) = &transport.bdt_persist_path {
            let _ = std::fs::remove_file(path);
        }
        match started {
            Err(Error::Transport(ref err)) if lost_port(attempt, err) => attempt += 1,
            started => return (transport, port, started),
        }
    }
}

/// A BBMD bound to `0.0.0.0`. `local` replaces the host's IPv4 addresses and
/// default-route address; `None` reads the real ones.
fn wildcard_bbmd(
    port: u16,
    bdt: Vec<BdtEntry>,
    local: Option<(Vec<Ipv4Addr>, Option<Ipv4Addr>)>,
) -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::UNSPECIFIED, port, Ipv4Addr::LOCALHOST);
    transport.enable_bbmd(bdt);
    transport.local_ipv4_for_test = local.map(Ok);
    transport
}

/// Write `rows` as a persisted BDT file unique to this process and `label`.
fn persisted_bdt(label: &str, rows: &[BdtEntry]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusty-bacnet-{label}-{}-{}.bdt",
        std::process::id(),
        rows.first().map_or(0, |row| row.port)
    ));
    let mut seed = BytesMut::new();
    bbmd::encode_bdt_entries(rows, &mut seed);
    std::fs::write(&path, &seed).unwrap();
    path
}

#[test]
fn wildcard_selection_takes_the_one_local_row_at_the_bound_port() {
    let local = [Ipv4Addr::LOCALHOST, LAN, OTHER_LAN];
    // A remote peer, a local address at another port and a repeated row do
    // not make the choice ambiguous, and the own row wins over the route.
    let rows = [
        row(REMOTE_PEER, PORT),
        row(LAN, PORT + 1),
        row(OTHER_LAN, PORT),
        row(OTHER_LAN, PORT),
    ];
    assert_eq!(select(&rows, &local, Some(LAN)).unwrap(), OTHER_LAN);
    // A loopback row is a local address like any other.
    assert_eq!(
        select(&[row(Ipv4Addr::LOCALHOST, PORT)], &local, Some(LAN)).unwrap(),
        Ipv4Addr::LOCALHOST
    );
}

#[test]
fn wildcard_selection_refuses_several_own_rows() {
    let rows = [row(LAN, PORT), row(Ipv4Addr::LOCALHOST, PORT)];
    let err = select(&rows, &[Ipv4Addr::LOCALHOST, LAN], Some(LAN)).unwrap_err();
    assert!(matches!(err, Error::Transport(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        text.contains("the configured BDT has rows for several local IPv4 addresses")
            && text.contains("(127.0.0.1:47808, 192.0.2.10:47808)")
            && text.contains("bind an explicit interface address"),
        "{text}"
    );
}

#[test]
fn wildcard_selection_without_own_row_needs_a_local_non_loopback_route() {
    let local = [Ipv4Addr::LOCALHOST, LAN];
    let peers = [row(REMOTE_PEER, PORT)];
    assert_eq!(select(&peers, &local, Some(LAN)).unwrap(), LAN);
    // No route, a loopback route, and a route address not on this host.
    for route in [None, Some(Ipv4Addr::LOCALHOST), Some(OTHER_LAN)] {
        let err = select(&peers, &local, route).unwrap_err();
        assert!(matches!(err, Error::Transport(_)), "{err:?}");
        let text = err.to_string();
        assert!(
            text.contains("has no row for a local IPv4 address at port 47808")
                && text.contains("bind an explicit interface address")
                && text.contains("add this BBMD's own row to the BDT"),
            "{route:?}: {text}"
        );
    }
}

#[test]
fn limited_broadcast_off_the_default_route_warns_for_a_wildcard_bbmd() {
    let warns = |interface: Ipv4Addr, own: Ipv4Addr, route_ip, broadcast| {
        let ctx = OwnAddressContext {
            interface,
            port: PORT,
            host: &[],
            route_ip,
        };
        warn_if_broadcast_may_leave_another_interface(&ctx, own, broadcast)
    };
    let wildcard = Ipv4Addr::UNSPECIFIED;
    assert!(warns(wildcard, LAN, Some(OTHER_LAN), Ipv4Addr::BROADCAST));
    assert!(warns(wildcard, LAN, None, Ipv4Addr::BROADCAST));
    // The default-route interface, a subnet broadcast address, or an explicit
    // interface leave no doubt about the interface.
    assert!(!warns(wildcard, LAN, Some(LAN), Ipv4Addr::BROADCAST));
    let subnet_broadcast = Ipv4Addr::new(192, 0, 2, 255);
    assert!(!warns(wildcard, LAN, Some(OTHER_LAN), subnet_broadcast));
    assert!(!warns(LAN, LAN, Some(OTHER_LAN), Ipv4Addr::BROADCAST));
}

#[tokio::test]
async fn wildcard_bbmd_uses_its_own_bdt_row_as_origin_and_local_mac() {
    let bdt_peer = udp().await;
    // The host's real addresses: 127.0.0.1 is local, and the default-route
    // address, whatever it is, must not win over the own row.
    let (mut bbmd, port, started) = start_on_free_port(|port| {
        wildcard_bbmd(
            port,
            vec![
                row(Ipv4Addr::LOCALHOST, port),
                row(Ipv4Addr::LOCALHOST, port_of(&bdt_peer)),
            ],
            None,
        )
    })
    .await;
    let mut rx = started.unwrap();
    let own = (Ipv4Addr::LOCALHOST.octets(), port);

    assert_eq!(bbmd.local_mac(), encode_bip_mac(own.0, own.1));
    {
        let state = bbmd.bbmd_state().unwrap().lock().unwrap();
        assert_eq!(state.local_address(), own);
        assert_eq!(state.bdt().len(), 2, "no second self row");
    }

    bbmd.send_broadcast(NPDU).await.unwrap();
    assert_own_forwarded(&recv_bvll(&bdt_peer).await, own, "BDT peer");
    // The local copy comes back from 127.0.0.1, the local MAC, so it is dropped
    // as the echo rather than delivered and forwarded a second time.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "own broadcast echo must not be delivered"
    );
    assert_no_bvll(&bdt_peer, "BDT peer").await;
    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn wildcard_bbmd_with_several_own_rows_fails_start_and_keeps_its_config() {
    // The second half gives the port back and takes it again, which another
    // process can win (#1032, #1068). A lost port reruns the test on a fresh
    // one; a start that really leaked its port would fail every attempt.
    for attempt in 1..=ATTEMPTS {
        let (mut bbmd, port, started) = start_on_free_port(|port| {
            wildcard_bbmd(
                port,
                vec![row(Ipv4Addr::LOCALHOST, port), row(LAN, port)],
                Some((vec![Ipv4Addr::LOCALHOST, LAN], Some(LAN))),
            )
        })
        .await;

        let err = started.unwrap_err();

        let text = err.to_string();
        assert!(
            text.contains("several local IPv4 addresses")
                && text.contains("bind an explicit interface address"),
            "{text}"
        );
        assert!(bbmd.socket.is_none() && bbmd.recv_task.is_none());
        assert!(bbmd.bbmd.is_none());
        assert!(
            bbmd.bbmd_config.is_some(),
            "a failed start keeps the BBMD configuration"
        );
        assert_eq!(bbmd.local_mac(), [0; 6]);

        // The failed start released its port: a socket without address sharing
        // can bind it, and a corrected retry starts on it.
        match std::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)) {
            Ok(socket) => drop(socket),
            Err(err) if lost_port(attempt, &err) => continue,
            Err(err) => panic!("a failed start releases its port: {err}"),
        }
        bbmd.local_ipv4_for_test = Some(Ok((vec![Ipv4Addr::LOCALHOST], Some(LAN))));
        let _rx = match bbmd.start().await {
            Ok(rx) => rx,
            Err(Error::Transport(ref err)) if lost_port(attempt, err) => continue,
            Err(err) => panic!("the corrected retry starts: {err}"),
        };
        assert_eq!(
            bbmd.local_mac(),
            encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), port)
        );
        bbmd.stop().await.unwrap();
        return;
    }
}

#[tokio::test]
async fn wildcard_bbmd_without_own_row_or_usable_route_fails_start() {
    for route in [None, Some(Ipv4Addr::LOCALHOST)] {
        let mut bbmd = wildcard_bbmd(
            0,
            vec![row(REMOTE_PEER, PORT)],
            Some((vec![Ipv4Addr::LOCALHOST], route)),
        );
        let text = bbmd.start().await.unwrap_err().to_string();
        assert!(
            text.contains("cannot determine its own B/IP address")
                && text.contains("add this BBMD's own row to the BDT"),
            "{route:?}: {text}"
        );
        assert!(bbmd.bbmd.is_none() && bbmd.bbmd_config.is_some());
    }
}

#[tokio::test]
async fn wildcard_start_without_a_listed_address_fails_with_or_without_a_bbmd() {
    // Every OS lists its addresses, with no bind-probe fallback: a failed
    // listing keeps its error kind, and an empty one is AddrNotAvailable.
    for listing_fails in [true, false] {
        let (kind, reason) = if listing_fails {
            (
                std::io::ErrorKind::Unsupported,
                "could not list the host's IPv4 addresses (no listing here)",
            )
        } else {
            (
                std::io::ErrorKind::AddrNotAvailable,
                "found no usable IPv4 address on this host",
            )
        };
        for bbmd in [false, true] {
            let mut transport = BipTransport::new(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::LOCALHOST);
            if bbmd {
                transport.enable_bbmd(vec![row(Ipv4Addr::LOCALHOST, PORT)]);
            }
            transport.local_ipv4_for_test = Some(if listing_fails {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "no listing here",
                ))
            } else {
                Ok((Vec::new(), Some(LAN)))
            });
            let Err(Error::Transport(err)) = transport.start().await else {
                panic!("bbmd {bbmd}: start must fail with a transport error");
            };
            let text = err.to_string();
            assert_eq!(err.kind(), kind, "bbmd {bbmd}: {text}");
            assert!(
                text.starts_with("a B/IP transport bound to 0.0.0.0 ")
                    && text.contains(reason)
                    && text.ends_with("; bind an explicit interface address instead"),
                "bbmd {bbmd}: {text}"
            );
            assert!(transport.socket.is_none() && transport.recv_task.is_none());
            assert_eq!(transport.bbmd_config.is_some(), bbmd);
        }
    }
}

#[tokio::test]
async fn wildcard_bbmd_reads_its_own_row_from_the_persisted_bdt() {
    // The configured BDT names the other local address; the persisted BDT is
    // the one the BBMD runs with, so its row decides.
    let (mut bbmd, port, started) = start_on_free_port(|port| {
        let mut bbmd = wildcard_bbmd(
            port,
            vec![row(LAN, port)],
            Some((vec![Ipv4Addr::LOCALHOST, LAN], Some(LAN))),
        );
        bbmd.set_bdt_persist_path(persisted_bdt("own-row", &[row(Ipv4Addr::LOCALHOST, port)]));
        bbmd
    })
    .await;
    let _rx = started.unwrap();

    assert_eq!(
        bbmd.local_mac(),
        encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), port)
    );
    let bdt = bbmd.bbmd_state().unwrap().lock().unwrap().bdt().to_vec();
    assert_eq!(bdt, vec![row(Ipv4Addr::LOCALHOST, port)]);
    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn wildcard_bbmd_does_not_fall_back_when_the_persisted_bdt_cannot_choose() {
    let (bbmd, _, started) = start_on_free_port(|port| {
        // The persisted BDT is valid but has two local rows at the bound port.
        let path = persisted_bdt(
            "ambiguous-own-row",
            &[row(Ipv4Addr::LOCALHOST, port), row(LAN, port)],
        );
        // The configured BDT alone would start, with LAN as the own row.
        let mut bbmd = wildcard_bbmd(
            port,
            vec![row(LAN, port)],
            Some((vec![Ipv4Addr::LOCALHOST, LAN], Some(LAN))),
        );
        bbmd.set_bdt_persist_path(path);
        bbmd
    })
    .await;

    let text = started.unwrap_err().to_string();
    let expected = format!(
        "the persisted BDT {} has rows for several local IPv4 addresses",
        bbmd.bdt_persist_path.as_ref().unwrap().display()
    );
    assert!(text.contains(&expected), "{text}");
    assert!(bbmd.socket.is_none() && bbmd.bbmd.is_none());
    assert!(bbmd.bbmd_config.is_some());
}

/// A wildcard BBMD started on an ephemeral port, with the default-route address
/// `LAN` as its own address. Returns the transport and the bound port.
async fn started_on_lan(
    foreign_devices: bool,
) -> (BipTransport, mpsc::Receiver<ReceivedNpdu>, u16) {
    let mut bbmd = wildcard_bbmd(0, Vec::new(), Some((vec![LAN], Some(LAN))));
    if foreign_devices {
        bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    }
    let rx = bbmd.start().await.unwrap();
    let port = bbmd.port;
    assert_eq!(bbmd.local_mac(), encode_bip_mac(LAN.octets(), port));
    (bbmd, rx, port)
}

#[tokio::test]
async fn restart_chooses_the_bbmd_address_again_and_drops_the_stale_self_row() {
    // Each run starts on a fresh port; see `restart` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let bdt_peer = udp().await;
        let foreign = udp().await;
        let (mut bbmd, _rx, port) = started_on_lan(true).await;
        let peer_row = row(Ipv4Addr::LOCALHOST, port_of(&bdt_peer));
        let loopback_row = row(Ipv4Addr::LOCALHOST, port);
        {
            // 127.0.0.1 is not among the host's addresses yet, so its row at
            // the bound port is not the BBMD's own, and a self row for LAN is
            // added.
            let mut state = bbmd.bbmd_state().unwrap().lock().unwrap();
            state
                .set_bdt(vec![peer_row.clone(), loopback_row.clone()])
                .unwrap();
            assert_eq!(
                state.bdt(),
                &[peer_row.clone(), loopback_row.clone(), row(LAN, port)]
            );
            assert_eq!(
                state.register_foreign_device([127, 0, 0, 1], port_of(&foreign), 60),
                BvlcResultCode::SUCCESSFUL_COMPLETION
            );
        }
        bbmd.stop().await.unwrap();

        // Restart: now 127.0.0.1 is local, so its row is the BBMD's own.
        bbmd.local_ipv4_for_test = Some(Ok((vec![Ipv4Addr::LOCALHOST, LAN], Some(LAN))));
        let Some(started) = restart(&mut bbmd, attempt).await else {
            continue;
        };
        let mut rx = started.unwrap();
        let own = (Ipv4Addr::LOCALHOST.octets(), port);
        assert_eq!(bbmd.local_mac(), encode_bip_mac(own.0, own.1));
        {
            let mut state = bbmd.bbmd_state().unwrap().lock().unwrap();
            assert_eq!(state.local_address(), own);
            // The self row appended for LAN is gone, not left behind as a peer.
            assert_eq!(state.bdt(), &[peer_row, loopback_row]);
            assert_eq!(state.fdt().len(), 1, "the FDT survives the restart");
        }

        bbmd.send_broadcast(NPDU).await.unwrap();
        assert_own_forwarded(&recv_bvll(&bdt_peer).await, own, "BDT peer");
        assert_own_forwarded(&recv_bvll(&foreign).await, own, "foreign device");
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err(),
            "own broadcast echo must not be delivered"
        );
        assert_no_bvll(&bdt_peer, "BDT peer").await;
        assert_no_bvll(&foreign, "foreign device").await;
        bbmd.stop().await.unwrap();
        return;
    }
}

#[tokio::test]
async fn failed_restart_keeps_the_bbmd_state_and_bdt() {
    // Each run starts on a fresh port; see `restart` for why it can lose it.
    // The failing restart binds before it chooses the address, so a lost port
    // would fail it with the wrong error.
    for attempt in 1..=ATTEMPTS {
        let (mut bbmd, _rx, port) = started_on_lan(true).await;
        let listed = vec![
            row(REMOTE_PEER, PORT),
            row(Ipv4Addr::LOCALHOST, port),
            row(OTHER_LAN, port),
        ];
        let (own, bdt) = {
            let mut state = bbmd.bbmd_state().unwrap().lock().unwrap();
            state.set_bdt(listed.clone()).unwrap();
            assert_eq!(
                state.register_foreign_device([127, 0, 0, 1], 47809, 60),
                BvlcResultCode::SUCCESSFUL_COMPLETION
            );
            (state.local_address(), state.bdt().to_vec())
        };
        assert_eq!(own, (LAN.octets(), port));
        bbmd.stop().await.unwrap();

        // Both listed rows at the bound port are now local: no choice is
        // possible.
        bbmd.local_ipv4_for_test = Some(Ok((vec![Ipv4Addr::LOCALHOST, OTHER_LAN], Some(LAN))));
        let Some(started) = restart(&mut bbmd, attempt).await else {
            continue;
        };
        let text = started.unwrap_err().to_string();
        assert!(
            text.contains("the BDT has rows for several local IPv4 addresses"),
            "{text}"
        );
        assert!(bbmd.socket.is_none() && bbmd.recv_task.is_none());
        {
            let mut state = bbmd.bbmd_state().unwrap().lock().unwrap();
            assert_eq!(state.local_address(), own);
            assert_eq!(state.bdt(), bdt.as_slice());
            assert_eq!(state.fdt().len(), 1);
        }

        // With one of them local again, the same transport restarts with it,
        // and the self row appended for LAN goes.
        bbmd.local_ipv4_for_test = Some(Ok((vec![Ipv4Addr::LOCALHOST], Some(LAN))));
        let Some(started) = restart(&mut bbmd, attempt).await else {
            continue;
        };
        let _rx = started.unwrap();
        {
            let state = bbmd.bbmd_state().unwrap().lock().unwrap();
            assert_eq!(state.local_address(), (Ipv4Addr::LOCALHOST.octets(), port));
            assert_eq!(state.bdt(), listed.as_slice());
        }
        bbmd.stop().await.unwrap();
        return;
    }
}

#[tokio::test]
async fn restart_that_would_overflow_the_bdt_fails_with_context_and_keeps_the_state() {
    // Each run starts on a fresh port; see `restart` for why it can lose it.
    // The restart binds before it chooses the address, so a lost port would
    // fail it with the wrong error.
    for attempt in 1..=ATTEMPTS {
        let (mut bbmd, _rx, port) = started_on_lan(false).await;
        // A full BDT that lists this BBMD's row for LAN, so none was appended.
        let mut full: Vec<BdtEntry> = (1..BbmdState::MAX_BDT_ENTRIES as u32)
            .map(|i| row(Ipv4Addr::from(0x0A00_0000 + i), PORT))
            .collect();
        full.push(row(LAN, port));
        bbmd.bbmd_state()
            .unwrap()
            .lock()
            .unwrap()
            .set_bdt(full.clone())
            .unwrap();
        bbmd.stop().await.unwrap();

        // LAN is gone and the route now leaves from OTHER_LAN, whose self row
        // does not fit.
        bbmd.local_ipv4_for_test = Some(Ok((vec![OTHER_LAN], Some(OTHER_LAN))));
        let Some(started) = restart(&mut bbmd, attempt).await else {
            continue;
        };
        let err = started.unwrap_err();

        assert!(matches!(err, Error::Encoding(_)), "{err:?}");
        let text = err.to_string();
        let moved = format!(
            "BBMD restart moved its own B/IP address from {LAN}:{port} to {OTHER_LAN}:{port}"
        );
        assert!(
            text.contains(&moved) && text.contains("would exceed the BDT limit of 128 entries"),
            "{text}"
        );
        assert!(bbmd.socket.is_none());
        let state = bbmd.bbmd_state().unwrap().lock().unwrap();
        assert_eq!(state.local_address(), (LAN.octets(), port));
        assert_eq!(state.bdt(), full.as_slice());
        return;
    }
}

/// Another thread holds the BBMD state's lock for the whole restart. A
/// restart that chose the address again would block on it, and the timeout
/// would fire on the other worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_interface_restart_keeps_the_bbmd_address_without_locking_the_state() {
    // Each run starts on a fresh port; see `restart` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
        bbmd.enable_bbmd(Vec::new());
        let _rx = bbmd.start().await.unwrap();
        let own = (Ipv4Addr::LOCALHOST.octets(), bbmd.port);
        bbmd.stop().await.unwrap();

        let state = Arc::clone(bbmd.bbmd_state().unwrap());
        let (locked_tx, locked) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let guard = state.lock().unwrap();
            locked_tx.send(guard.local_address()).unwrap();
            let _ = released.recv();
        });
        let held_address = locked.recv().unwrap();
        let mut restarting = tokio::spawn(async move {
            let started = restart(&mut bbmd, attempt).await;
            (bbmd, started)
        });
        let outcome = tokio::time::timeout(Duration::from_secs(2), &mut restarting).await;
        release.send(()).unwrap();
        holder.join().unwrap();
        let (mut bbmd, restarted) = outcome
            .expect("an explicit-interface restart does not lock the BBMD state")
            .unwrap();
        let Some(started) = restarted else {
            continue;
        };
        let _rx = started.unwrap();
        assert_eq!(held_address, own);
        assert_eq!(bbmd.local_mac(), encode_bip_mac(own.0, own.1));
        bbmd.stop().await.unwrap();
        return;
    }
}
