//! Opt-in actual Linux Ethernet fixture; requires an isolated peer and NET_RAW.
#![cfg(all(target_os = "linux", feature = "ethernet"))]
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_objects::{database::ObjectDatabase, network_port::NetworkPortObject};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_transport::{
    any::AnyTransport, ethernet::EthernetTransport, mstp::LoopbackSerial, port::TransportPort,
};
use bacnet_types::{enums::NetworkType, MacAddr};
use std::{future::Future, path::PathBuf, time::Duration};

fn interface() -> String {
    std::env::var("BACNET_ETHERNET_INTERFACE").expect("explicit isolated interface")
}
fn raw_sockets() -> usize {
    // Count this test process's packet FDs, excluding namespace daemons/peers.
    let packet_inodes: std::collections::HashSet<_> = std::fs::read_to_string("/proc/net/packet")
        .unwrap()
        .lines()
        .skip(1)
        .filter_map(|row| {
            row.split_whitespace()
                .last()
                .map(|inode| format!("socket:[{inode}]"))
        })
        .collect();
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .filter(|link| packet_inodes.contains(link.to_string_lossy().as_ref()))
        .count()
}

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("isolated Ethernet fixture made no progress");
}
#[tokio::test]
#[ignore = "requires explicit isolated Linux interface and CAP_NET_RAW"]
async fn ethernet_transport_drop_releases_socket() {
    let before = raw_sockets();
    let mut transport = EthernetTransport::new(&interface());
    let mut received = transport.start().await.unwrap();
    assert_eq!(raw_sockets(), before + 1);
    drop(transport);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await
            .expect("drop must close receive owner")
            .is_none()
    );
    assert_eq!(raw_sockets(), before);
}
#[tokio::test]
#[ignore = "requires explicit isolated Linux interface and CAP_NET_RAW"]
async fn ethernet_transport_cancelled_stop_joins_before_return() {
    let before = raw_sockets();
    let mut transport = EthernetTransport::new(&interface());
    let mut received = transport.start().await.unwrap();
    {
        let stop = transport.stop();
        tokio::pin!(stop);
        // Poll once on this current-thread runtime: abort starts, join is pending.
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(stop.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }
    transport.stop().await.unwrap();
    assert_eq!(
        raw_sockets(),
        before,
        "completed stop must join the raw-fd owner"
    );
    assert!(received.recv().await.is_none());
}

/// A raw socket on the transport's interface. A Linux bridge drops a frame
/// whose source MAC is a group address, so no peer container could deliver
/// one; but every frame this socket sends also reaches the transport's
/// socket on the same interface, as a copy of outgoing traffic, with
/// whatever source it names.
struct RawLink {
    fd: std::os::fd::OwnedFd,
    if_index: i32,
}

impl RawLink {
    #[allow(unsafe_code)]
    fn open() -> Self {
        use std::os::fd::FromRawFd;
        let protocol = (libc::ETH_P_ALL as u16).to_be();
        // SAFETY: plain socket(2); a non-negative result is a fresh fd.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, i32::from(protocol)) };
        assert!(fd >= 0, "{}", std::io::Error::last_os_error());
        // SAFETY: `fd` was just opened and is owned by nothing else.
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        let name = std::ffi::CString::new(interface()).unwrap();
        // SAFETY: `name` is a valid NUL-terminated string.
        let if_index = unsafe { libc::if_nametoindex(name.as_ptr()) } as i32;
        assert!(if_index > 0, "unknown interface");
        let timeout = libc::timeval {
            tv_sec: 0,
            tv_usec: 200_000,
        };
        // SAFETY: the option value points to a live timeval of the size passed.
        let set = unsafe {
            libc::setsockopt(
                std::os::fd::AsRawFd::as_raw_fd(&fd),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&timeout as *const libc::timeval).cast(),
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        assert_eq!(set, 0, "{}", std::io::Error::last_os_error());
        Self { fd, if_index }
    }

    /// Send an 802.3/LLC frame from `source` to `destination`.
    #[allow(unsafe_code)]
    fn send(&self, destination: [u8; 6], source: [u8; 6], control: u8, payload: &[u8]) {
        let mut frame = destination.to_vec();
        frame.extend_from_slice(&source);
        frame.extend_from_slice(&(payload.len() as u16 + 3).to_be_bytes());
        frame.extend_from_slice(&[0x82, 0x82, control]);
        frame.extend_from_slice(payload);
        frame.resize(frame.len().max(60), 0);
        // SAFETY: an all-zero sockaddr_ll is valid; the fields set below
        // name the interface and the destination.
        let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        address.sll_family = libc::AF_PACKET as u16;
        address.sll_ifindex = self.if_index;
        address.sll_halen = 6;
        address.sll_addr[..6].copy_from_slice(&destination);
        // SAFETY: the buffer and the address are live for the call, with
        // the lengths passed.
        let sent = unsafe {
            libc::sendto(
                std::os::fd::AsRawFd::as_raw_fd(&self.fd),
                frame.as_ptr().cast(),
                frame.len(),
                0,
                (&address as *const libc::sockaddr_ll).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        assert_eq!(sent, frame.len() as isize);
    }

    /// The next frame on the interface sent from `source`, within the
    /// fixture's progress guard.
    #[allow(unsafe_code)]
    fn next_from(&self, source: &[u8]) -> Vec<u8> {
        let until = std::time::Instant::now() + Duration::from_secs(30);
        let mut buffer = [0u8; 2048];
        while std::time::Instant::now() < until {
            // SAFETY: the buffer is live for the call, with its length.
            let read = unsafe {
                libc::recv(
                    std::os::fd::AsRawFd::as_raw_fd(&self.fd),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    0,
                )
            };
            if read >= 12 && buffer[6..12] == *source {
                return buffer[..read as usize].to_vec();
            }
        }
        panic!("isolated Ethernet fixture made no progress");
    }
}

/// A frame whose source MAC is a group address is dropped before its XID or
/// TEST command is answered or its NPDU is handed up, and counted (#1492);
/// one from a station still gets through.
#[tokio::test]
#[ignore = "requires explicit isolated Linux interface and CAP_NET_RAW"]
async fn ethernet_transport_drops_group_source_frames() {
    let mut transport = EthernetTransport::new(&interface());
    let mut received = transport.start().await.unwrap();
    let local: [u8; 6] = transport.local_mac().try_into().unwrap();
    let station = [0x02, 0, 0, 0, 0, 0x77];
    let query = [1, 0x80, 0x12];
    let link = tokio::task::spawn_blocking(move || {
        let link = RawLink::open();
        for group in [[0xFF; 6], [0x01, 0x00, 0x5E, 0x00, 0x00, 0x42]] {
            link.send(local, group, 0xE3, &[b'G'; 43]);
            link.send(local, group, 0xAF, &[0x81, 0x01, 0x01]);
            link.send(local, group, 0x03, &query);
        }
        link.send(local, station, 0xE3, &[b'H'; 43]);
        link.send(local, station, 0x03, &query);
        // The first frame the transport sends answers the station's TEST.
        let answer = link.next_from(&local);
        assert_eq!(answer[..6], station, "answered a group source");
        assert_eq!(answer[14..17], [0x82, 0x83, 0xF3]);
        assert_eq!(answer[17..17 + 43], [b'H'; 43]);
        link
    })
    .await
    .unwrap();
    let npdu = tokio::time::timeout(Duration::from_secs(30), received.recv())
        .await
        .expect("isolated Ethernet fixture made no progress")
        .unwrap();
    assert_eq!(npdu.source_mac[..], station, "handed up a group source");
    assert_eq!(npdu.npdu[..], query);
    assert_eq!(transport.group_source_drops(), 6);
    drop(link);
    transport.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated Linux raw peer; see ethernet_network_numbers/README.md"]
async fn ethernet_number_full_server_and_client_wire() {
    let directory = PathBuf::from(
        std::env::var("BACNET_ETHERNET_FIXTURE_DIR").expect("peer coordination directory"),
    );
    let before = raw_sockets();
    for case in ["server-stop", "server-drop", "client-stop", "client-drop"] {
        let transport: AnyTransport<LoopbackSerial> =
            AnyTransport::Ethernet(EthernetTransport::new(&interface()));
        assert!(transport.bip_port().is_none());
        let is_server = case.starts_with("server");
        let (mut server, mut client) = if is_server {
            let mut db = ObjectDatabase::new();
            db.add(Box::new(
                NetworkPortObject::new_non_bip(
                    9,
                    "unrelated configured Ethernet",
                    NetworkType::ETHERNET,
                    999,
                    MacAddr::from_slice(&[2, 0, 0, 0, 0, 9]),
                    1476,
                )
                .unwrap(),
            ))
            .unwrap();
            (
                Some(
                    BACnetServer::start(ServerConfig::default(), db, transport)
                        .await
                        .unwrap(),
                ),
                None,
            )
        } else {
            (
                None,
                Some(
                    BACnetClient::start(ClientConfig::default(), transport)
                        .await
                        .unwrap(),
                ),
            )
        };
        let mac = if let Some(server) = &server {
            server.local_mac()
        } else {
            client.as_ref().unwrap().local_mac()
        };
        let ready = serde_json::json!({"case":case,"mac":mac,"raw_sockets":raw_sockets()});
        let temporary = directory.join(format!("{case}.tmp"));
        std::fs::write(&temporary, ready.to_string()).unwrap();
        std::fs::rename(temporary, directory.join(format!("{case}.ready"))).unwrap();
        wait_for(|| {
            directory.join(format!("{case}.done")).exists() || directory.join("peer.error").exists()
        })
        .await;
        assert!(
            !directory.join("peer.error").exists(),
            "independent peer assertions failed"
        );
        if case.ends_with("stop") {
            if let Some(server) = &mut server {
                server.stop().await.unwrap();
            }
            if let Some(client) = &mut client {
                client.stop().await.unwrap();
            }
            assert_eq!(
                raw_sockets(),
                before,
                "stop releases raw FD before owner drop"
            );
        }
        drop(server);
        drop(client);
        wait_for(|| raw_sockets() == before).await;
        std::fs::write(
            directory.join(format!("{case}.stopped")),
            b"raw_fds_released",
        )
        .unwrap();
        wait_for(|| directory.join(format!("{case}.checked")).exists()).await;
    }
}
