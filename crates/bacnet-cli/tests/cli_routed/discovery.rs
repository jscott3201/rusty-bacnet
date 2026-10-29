//! Causal coverage for fresh routed CLI clients sharing one source endpoint.
use super::*;
use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::{NpduAddress, decode_npdu};
use bacnet_server::server::{DiscoveryPolicy, ServerConfig};
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer};
use bacnet_types::enums::UnconfirmedServiceChoice;
use futures_util::FutureExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::paused::{self, BlockingGuard};

type Sources = Arc<Mutex<Vec<NpduAddress>>>;

// This relay records the actual routed NPDU on the station's incoming link.
// Its endpoint MACs preserve the router/station link identities.
struct Relay(Option<tokio::task::JoinHandle<()>>);
impl Drop for Relay {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

async fn observed_site(
    database: bacnet_objects::database::ObjectDatabase,
    config: ServerConfig,
) -> (
    BACnetRouter,
    BACnetServer<LoopbackTransport>,
    u16,
    Sources,
    FrameTapReceiver,
    Relay,
) {
    for _ in 0..8 {
        let port = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut bip = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
        bip.set_mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(ForeignDevicePolicy::default()),
            ..Default::default()
        }));
        let tap = bip.frame_tap().subscribe();
        let (router_end, mut incoming) = LoopbackTransport::pair(vec![1], vec![5]);
        let (mut outgoing, station) = LoopbackTransport::pair(vec![1], vec![5]);
        let mut to_station = incoming.start().await.unwrap();
        let mut to_router = outgoing.start().await.unwrap();
        let Ok(StartedRouter { router, .. }) = BACnetRouter::start(
            vec![
                RouterPort {
                    transport: AnyTransport::Bip(Box::new(bip)),
                    network_number: 1,
                },
                RouterPort {
                    transport: AnyTransport::Loopback(router_end),
                    network_number: DNET,
                },
            ],
            RouterOptions::new(),
        )
        .await
        else {
            // Another test took the probed port before the router bound it.
            continue;
        };
        let sources = Sources::default();
        let observed = sources.clone();
        let relay = Relay(Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(frame) = to_station.recv() => {
                        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
                        if let Ok(Apdu::UnconfirmedRequest(request)) = decode_apdu(npdu.payload)
                            && request.service_choice == UnconfirmedServiceChoice::WHO_IS {
                            observed.lock().unwrap().push(npdu.source.expect("routed source"));
                        }
                        if frame.link_layer_group {
                            outgoing.send_broadcast(&frame.npdu).await.unwrap();
                        } else {
                            outgoing.send_unicast(&frame.npdu, &[5]).await.unwrap();
                        }
                    }
                    Some(frame) = to_router.recv() => {
                        incoming.send_unicast(&frame.npdu, &[1]).await.unwrap();
                    }
                    else => break,
                }
            }
        })));
        let server = BACnetServer::start(config, database, station)
            .await
            .unwrap();
        return (router, server, port, sources, tap, relay);
    }
    panic!("no loopback UDP port the router could bind");
}

async fn query(port: u16, source_port: u16) -> std::process::Output {
    let mut cmd = command();
    cmd.args([
        "--interface",
        "127.0.0.1",
        "--port",
        &source_port.to_string(),
        "--foreign-bbmd",
        &format!("127.0.0.1:{port}"),
        "--json",
        "read",
        "2:0x05",
        "analog-input:2",
        "present-value",
    ]);
    paused::output(Process::spawn(&mut cmd)).await
}

fn source_port(source: &NpduAddress) -> u16 {
    assert_eq!(source.network, 1);
    assert_eq!(&source.mac_address[..4], &[127, 0, 0, 1]);
    assert_eq!(source.mac_address.len(), 6);
    u16::from_be_bytes([source.mac_address[4], source.mac_address[5]])
}

fn check(server: &BACnetServer<LoopbackTransport>, received: u64, coalesced: u64, sent: u64) {
    let c = server.discovery_counters();
    assert_eq!(c.who_is_received, received);
    assert_eq!(c.requests_coalesced, coalesced);
    assert_eq!(c.i_am_sent, sent);
    assert_eq!(c.responses_throttled_global, 0);
    assert_eq!(c.responses_throttled_source, 0);
}

#[tokio::test(start_paused = true)]
async fn default_policy_coalesces_a_reused_routed_cli_source() {
    let _clock = BlockingGuard::hold_clock().await;
    let start = tokio::time::Instant::now();
    let (mut router, mut server, port, sources, mut tap, mut relay) = observed_site(
        read_services::database(),
        ServerConfig {
            device: lab_identity(INSTANCE),
            discovery_policy: DiscoveryPolicy::default(),
            ..Default::default()
        },
    )
    .await;
    let first = query(port, 0).await;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    check(&server, 1, 0, 1);
    let first_source = sources.lock().unwrap()[0].clone();
    let reused_port = source_port(&first_source);
    let second = query(port, reused_port).await;
    failure(&second, "did not answer a Who-Is on network 2");
    check(&server, 2, 1, 1);
    assert_eq!(sources.lock().unwrap()[1], first_source);
    assert_eq!(tokio::time::Instant::now(), start);

    // Reserving the old source excludes it from the next port-0 choice. The
    // observed routed source proves the distinct child's actual identity.
    let held = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, reused_port)).unwrap();
    let distinct = query(port, 0).await;
    assert!(
        distinct.status.success(),
        "{}",
        String::from_utf8_lossy(&distinct.stderr)
    );
    check(&server, 3, 1, 2);
    let distinct_source = sources.lock().unwrap()[2].clone();
    assert_ne!(distinct_source, first_source);
    assert_eq!(tokio::time::Instant::now(), start);
    drop(held);

    tokio::time::advance(Duration::from_millis(200)).await;
    let boundary = query(port, reused_port).await;
    assert!(
        boundary.status.success(),
        "{}",
        String::from_utf8_lossy(&boundary.stderr)
    );
    check(&server, 4, 1, 3);
    assert_eq!(sources.lock().unwrap().len(), 4);
    assert_eq!(sources.lock().unwrap()[3], first_source);
    assert_eq!(
        tokio::time::Instant::now(),
        start + Duration::from_millis(200)
    );
    let mut inbound_sources = Vec::new();
    while let Some(frame) = tap.try_recv() {
        if frame.direction == TapDirection::Inbound
            && let TapPeer::Udp(peer) = frame.peer
        {
            inbound_sources.push(peer.port());
        }
    }
    assert_eq!(tap.dropped(), 0);
    assert!(inbound_sources.contains(&reused_port));
    assert!(inbound_sources.contains(&source_port(&distinct_source)));
    eprintln!(
        "actual routed sources: {:?}; B/IP inbound ports: {inbound_sources:?}; router: {:?}",
        sources.lock().unwrap(),
        router.counters()
    );
    assert_eq!(router.counters(), Default::default());
    server.stop().await.unwrap();
    router.stop().await.unwrap();
    let task = relay.0.take().unwrap();
    task.abort();
    match task.await {
        Ok(()) => {}
        Err(error) => assert!(error.is_cancelled()),
    }
}

#[tokio::test(start_paused = true)]
async fn service_fixture_preserves_full_lifecycle_for_reused_sources() {
    let _clock = BlockingGuard::hold_clock().await;
    let start = tokio::time::Instant::now();
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let config = service_config(ServerConfig {
            device: lab_identity(INSTANCE),
            ..Default::default()
        });
        let (mut router, mut server, port, sources, _tap, mut relay) =
            observed_site(object_alarm::database(), config).await;
        // No retry or assumed reservation: every successful child must report
        // this exact source in the routed NPDU observed at the server link.
        let source = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let result = std::panic::AssertUnwindSafe(object_alarm::object_lifecycle(
            &server,
            Some(port),
            &target,
            Some(source),
        ))
        .catch_unwind()
        .await;
        eprintln!(
            "{target}: observed sources {:?}; counters {:?}",
            sources.lock().unwrap(),
            server.discovery_counters()
        );
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
        assert_eq!(tokio::time::Instant::now(), start);
        {
            let observed = sources.lock().unwrap();
            assert_eq!(observed.len(), 5);
            for identity in observed.iter() {
                assert_eq!(source_port(identity), source);
            }
            eprintln!("{target}: five fresh lifecycle children used {observed:?}");
        }
        check(&server, 5, 0, 5);
        server.stop().await.unwrap();
        router.stop().await.unwrap();
        let task = relay.0.take().unwrap();
        task.abort();
        match task.await {
            Ok(()) => {}
            Err(error) => assert!(error.is_cancelled()),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn paused_child_wait_reaps_and_removes_outputs_on_cancellation_and_panic() {
    let _clock = BlockingGuard::hold_clock().await;
    let held = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let bbmd = held.local_addr().unwrap().to_string();
    for panic in [false, true] {
        let mut cmd = command();
        cmd.args([
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--foreign-bbmd",
            &bbmd,
            "read",
            "2:1234",
            "analog-input:2",
            "present-value",
        ]);
        let process = Process::spawn(&mut cmd);
        let (entered, started) = tokio::sync::oneshot::channel();
        let (fail, failed) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = paused::output_started(process, entered) => panic!("child exited before owner cleanup"),
                _ = failed => panic!("intentional subprocess owner panic"),
            }
        });
        let directory = started.await.unwrap();
        assert!(directory.exists());
        if panic {
            fail.send(()).unwrap();
        } else {
            task.abort();
        }
        let error = task.await.unwrap_err();
        assert_eq!(error.is_panic(), panic);
        assert_eq!(error.is_cancelled(), !panic);
        // Process::drop kills AND waits before Files::drop removes this path.
        // The async owner cannot retire until that blocking body acknowledges.
        assert!(
            !directory.exists(),
            "child owner left output resources behind"
        );
    }
}
