//! A lab device on loopback and a counting relay in front of it, for the
//! one-shot tests that say what a command costs on the wire.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use super::lab::{lab_database, lab_identity};

/// The lab device, and its address.
pub async fn server() -> (BACnetServer<BipTransport>, SocketAddr) {
    serving(lab_database(1234)).await
}

/// Device 1234 serving `database`, and its address.
pub async fn serving(database: ObjectDatabase) -> (BACnetServer<BipTransport>, SocketAddr) {
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(database)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, u16::from_be_bytes([mac[4], mac[5]])));
    (server, address)
}

/// A B/IP relay on loopback in front of `device` that counts the distinct
/// confirmed requests it sees, and passes on the first `answered` of them
/// (and their retries), dropping the rest, so the device goes silent after
/// them. Returns the CLI's target, the count and the relay's task.
pub async fn relay(
    device: SocketAddr,
    answered: usize,
) -> (String, Arc<Mutex<HashSet<u8>>>, JoinHandle<()>) {
    let front = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = front.local_addr().unwrap().to_string();
    let seen: Arc<Mutex<HashSet<u8>>> = Arc::default();
    let log = Arc::clone(&seen);
    let task = tokio::spawn(async move {
        let mut cli = None;
        let mut passed = HashSet::new();
        let (mut up, mut down) = ([0u8; 1500], [0u8; 1500]);
        loop {
            tokio::select! {
                received = front.recv_from(&mut up) => {
                    let Ok((length, from)) = received else { return };
                    cli = Some(from);
                    let frame = &up[..length];
                    if let Some(npdu) = frame.get(4..).and_then(|n| decode_npdu(Bytes::copy_from_slice(n)).ok())
                        && let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload)
                    {
                        log.lock().unwrap().insert(request.invoke_id);
                        if !passed.contains(&request.invoke_id) {
                            if passed.len() == answered {
                                continue;
                            }
                            passed.insert(request.invoke_id);
                        }
                    }
                    let _ = back.send_to(frame, device).await;
                }
                received = back.recv_from(&mut down) => {
                    let Ok((length, _)) = received else { return };
                    if let Some(cli) = cli {
                        let _ = front.send_to(&down[..length], cli).await;
                    }
                }
            }
        }
    });
    (target, seen, task)
}
