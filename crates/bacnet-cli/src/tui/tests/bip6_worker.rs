//! The worker on a real B/IPv6 client on `::1`.
//!
//! The status line names the client's VMAC and the address it is bound to.
//! A directed Who-Is to an `[ip]:port` is resolved to the VMAC there (U.5)
//! beside the worker's loop, so the loop answers Cancel while the lookup
//! waits. Both ends have a device instance: a random VMAC would need a
//! multicast probe at start, which Linux loopback can't carry.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, UdpSocket};
use std::time::{Duration, Instant};

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::port::TransportPort;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::core::target::AddressStyle;
use crate::tui::message::{Command, DirectedTo, OpOutcome, WhoIsScope, WhoIsSpec, WorkerEvent};
use crate::tui::worker::{session, EventSink, EVENT_CHANNEL_CAPACITY};

/// Long enough for any step here, short of a hang.
const STEP: Duration = Duration::from_secs(10);

fn directed(peer: SocketAddrV6) -> WhoIsSpec {
    WhoIsSpec {
        scope: WhoIsScope::Directed {
            to: DirectedTo::Bip6(peer),
            label: peer.to_string(),
        },
        range: None,
        listen: Duration::from_millis(200),
    }
}

/// The next event that isn't about the device table.
async fn next_event(events: &mut mpsc::Receiver<WorkerEvent>) -> WorkerEvent {
    loop {
        let event = timeout(STEP, events.recv())
            .await
            .expect("an event in time")
            .expect("the worker is running");
        match event {
            WorkerEvent::Discovered(_)
            | WorkerEvent::Updated(_)
            | WorkerEvent::Snapshot(_)
            | WorkerEvent::Collision { .. } => {}
            other => return other,
        }
    }
}

#[tokio::test]
async fn a_directed_bip6_who_is_resolves_beside_the_loop() {
    let mut server = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1234));
    let mut server_rx = server.start().await.unwrap();
    let server_address = server.local_address().unwrap();
    let silent = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V6(silent_address) = silent.local_addr().unwrap() else {
        panic!("bound on ::1");
    };
    let client = BACnetClient::bip6_builder()
        .interface(Ipv6Addr::LOCALHOST)
        .port(0)
        .device_instance(4321)
        .build()
        .await
        .unwrap();
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (commands, command_rx) = mpsc::channel(16);
    let worker = tokio::spawn(Box::pin(session::serve(
        client,
        AddressStyle::Bip6,
        sink,
        command_rx,
        Default::default(),
    )));

    // The status line: the VMAC instance 4321 gives (H.7.2) and where the
    // client is bound.
    let WorkerEvent::Connected { local } = next_event(&mut events).await else {
        panic!("expected Connected first");
    };
    assert!(local.starts_with("bip6:0x0010E1 ([::1]:"), "{local}");
    assert!(local.ends_with(')'), "{local}");
    // The write session opens beside the loop; a flags-only one can't arm.
    assert!(matches!(
        next_event(&mut events).await,
        WorkerEvent::Safety(crate::tui::safety::SafetyEvent::Ready {
            workspace: false,
            ..
        })
    ));

    // Nobody answers this lookup, yet Cancel ends it at once, well inside
    // the three seconds the lookup may wait.
    let asked = Instant::now();
    let spec = directed(silent_address);
    for command in [Command::WhoIs { op: 1, spec }, Command::Cancel { op: 1 }] {
        commands.send(command).await.unwrap();
    }
    assert_eq!(
        next_event(&mut events).await,
        WorkerEvent::OpFinished {
            op: 1,
            outcome: OpOutcome::Cancelled
        }
    );
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "{:?}",
        asked.elapsed()
    );

    // The server answers: the Who-Is goes out to its VMAC.
    let spec = directed(server_address);
    commands.send(Command::WhoIs { op: 2, spec }).await.unwrap();
    assert_eq!(
        next_event(&mut events).await,
        WorkerEvent::WhoIsSent { op: 2 }
    );
    let who_is = timeout(STEP, server_rx.recv()).await.unwrap().unwrap();
    assert_eq!(who_is.source_mac.as_slice(), [0x00, 0x10, 0xE1]);
    assert!(who_is.npdu.ends_with(&[0x10, 0x08]), "{:02x?}", who_is.npdu);
    assert_eq!(
        next_event(&mut events).await,
        WorkerEvent::OpFinished {
            op: 2,
            outcome: OpOutcome::Completed
        }
    );

    // A lookup nobody answers fails the Who-Is with the transport's reason.
    let spec = directed(silent_address);
    commands.send(Command::WhoIs { op: 3, spec }).await.unwrap();
    match next_event(&mut events).await {
        WorkerEvent::OpFinished {
            op: 3,
            outcome: OpOutcome::Failed(reason),
        } => assert!(reason.contains("Virtual-Address-Resolution"), "{reason}"),
        other => panic!("expected the lookup to fail, got {other:?}"),
    }

    drop(commands);
    timeout(STEP, worker).await.unwrap().unwrap();
    server.stop().await.unwrap();
}
