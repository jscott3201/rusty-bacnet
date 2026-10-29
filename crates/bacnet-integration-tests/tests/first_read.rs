//! Execute the exact displayed main, optionally using an explicit registry build.
#![forbid(unsafe_code)]

#[path = "first_read/process.rs"]
mod process;
#[path = "first_read/remote_errors.rs"]
mod remote_errors;

use bacnet_bip_codec::Bvll;
use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::tap::{TapDirection, TapPeer, TappedFrame};
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use process::{Process, WAIT};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use tokio::sync::{oneshot, watch};
use tokio::time::timeout;

struct Server {
    address: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    done: watch::Receiver<Option<Result<(), String>>>,
    frames: bacnet_transport::tap::FrameTapReceiver,
}

impl Server {
    async fn start(value: Option<f32>) -> Self {
        let mut database = ObjectDatabase::new();
        database
            .add(Box::new(
                DeviceObject::new(DeviceConfig {
                    identity: DeviceIdentity::new(1234, 555).unwrap(),
                    ..DeviceConfig::default()
                })
                .unwrap(),
            ))
            .unwrap();
        for (instance, value) in [(1, value), (2, Some(-12.25))] {
            if let Some(value) = value {
                let mut ai = AnalogInputObject::new(
                    instance,
                    format!("First read fixture {instance}"),
                    EngineeringUnits::DEGREES_CELSIUS,
                )
                .unwrap();
                ai.set_present_value(value);
                database.add(Box::new(ai)).unwrap();
            }
        }
        let (stop, stopped) = oneshot::channel();
        let (done, result) = watch::channel(None);
        let (started, ready) = oneshot::channel();
        // The server's owner also survives loss of the caller's runtime.
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(async move {
                let mut server = BACnetServer::bip_builder()
                    .device(DeviceIdentity::new(1234, 555).unwrap())
                    .interface(Ipv4Addr::LOCALHOST)
                    .port(0)
                    .database(database)
                    .build()
                    .await
                    .map_err(|e| e.to_string())?;
                let address: SocketAddr = bacnet_bip_codec::address_from_slice(server.local_mac())
                    .unwrap()
                    .into();
                let frames = server.transport().unwrap().frame_tap().subscribe();
                let _ = started.send((address, frames));
                let _ = stopped.await;
                server.stop().await.map_err(|e| e.to_string())
            });
            drop(runtime);
            done.send_replace(Some(result));
        });
        let (address, frames) = ready.await.expect("server must report readiness");
        Self {
            address,
            stop: Some(stop),
            done: result,
            frames,
        }
    }

    async fn close(mut self) -> Vec<TappedFrame> {
        self.stop.take().unwrap().send(()).unwrap();
        timeout(WAIT, self.done.wait_for(Option::is_some))
            .await
            .unwrap()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .unwrap();
        assert_rebind(self.address);
        std::iter::from_fn(|| self.frames.try_recv()).collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn assert_rebind(address: SocketAddr) {
    let rebound = UdpSocket::bind(address).expect("retired endpoint must be immediately reusable");
    assert_eq!(rebound.local_addr().unwrap(), address);
}

fn request_oracle(frames: &[TappedFrame]) -> SocketAddr {
    let requests: Vec<_> = frames
        .iter()
        .filter(|frame| frame.direction == TapDirection::Inbound)
        .filter_map(|frame| {
            let Bvll::OriginalUnicastNpdu(npdu) = bacnet_bip_codec::decode(&frame.bytes).unwrap()
            else {
                return None;
            };
            let npdu = decode_npdu(Bytes::copy_from_slice(npdu)).unwrap();
            assert!(npdu.destination.is_none(), "direct local destination");
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                return None;
            };
            assert_eq!(
                request.service_choice,
                ConfirmedServiceChoice::READ_PROPERTY
            );
            let read = ReadPropertyRequest::decode(&request.service_request).unwrap();
            assert_eq!(
                read.object_identifier,
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
            );
            assert_eq!(read.property_identifier, PropertyIdentifier::PRESENT_VALUE);
            assert_eq!(read.property_array_index, None);
            let TapPeer::Udp(peer) = frame.peer else {
                panic!("expected UDP source");
            };
            Some(peer)
        })
        .collect();
    assert_eq!(requests.len(), 1, "one directed ReadProperty: {frames:?}");
    let peer = requests[0];
    assert!(peer.ip().is_loopback());
    assert_ne!(peer.port(), 0);
    // The guide binds port zero. The observed source and immediate rebind prove
    // the endpoint it actually used is retired when the process exits.
    assert_rebind(peer);
    peer
}

#[test]
fn executable_selection_is_explicit_and_has_no_missing_path_fallback() {
    let expected = std::path::Path::new(env!("CARGO_BIN_EXE_guide-first-read"))
        .canonicalize()
        .unwrap();
    assert_eq!(process::select_binary(None).unwrap(), expected);
    assert_eq!(
        process::select_binary(Some(expected.as_os_str())).unwrap(),
        expected
    );
    assert!(process::select_binary(Some(expected.join("missing-child").as_os_str())).is_err());
    assert!(process::select_binary(Some(expected.parent().unwrap().as_os_str())).is_err());
}

#[tokio::test]
async fn missing_and_malformed_arguments_fail_before_a_read() {
    for (arguments, error) in [
        (vec![], "usage: first-read <IPv4:port>"),
        (vec!["not-an-address"], "AddrParseError"),
        (vec!["127.0.0.1:99999"], "AddrParseError"),
        (vec!["[::1]:47808"], "AddrParseError"),
    ] {
        let output = Process::spawn(&process::binary(), &arguments)
            .unwrap()
            .finish()
            .await;
        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{output:?}"
        );
    }
}

#[tokio::test]
async fn actual_main_reads_typed_value_from_analog_input_one() {
    for value in [72.5, 18.75] {
        let server = Server::start(Some(value)).await;
        let output = Process::spawn(&process::binary(), &[&server.address.to_string()])
            .unwrap()
            .finish()
            .await;
        let frames = server.close().await;
        request_oracle(&frames);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, format!("Real({value})\n").as_bytes());
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[tokio::test]
async fn actual_remote_unknown_object_is_not_success_or_timeout() {
    let server = Server::start(None).await;
    let output = Process::spawn(&process::binary(), &[&server.address.to_string()])
        .unwrap()
        .finish()
        .await;
    let frames = server.close().await;
    request_oracle(&frames);
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("UnknownObject")
            || stderr.contains("unknown-object")
            || stderr.contains("UNKNOWN_OBJECT"),
        "{stderr}"
    );
    assert!(!stderr.to_lowercase().contains("timeout"), "{stderr}");
}

#[tokio::test]
async fn dropped_cancelled_and_panicking_callers_reap_the_child() {
    for mode in ["drop", "cancel", "panic"] {
        let peer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let child = Process::spawn(
            &process::binary(),
            &[&peer.local_addr().unwrap().to_string()],
        )
        .unwrap();
        let mut retired = child.retirement();
        let mut bytes = [0; 2048];
        let (_, source) = timeout(WAIT, peer.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        // Receiving its request proves the child is alive and owns this socket.
        assert!(UdpSocket::bind(source).is_err());
        match mode {
            "drop" => drop(child),
            "cancel" => {
                let (started, ready) = oneshot::channel();
                let task = tokio::spawn(async move {
                    let _ = started.send(());
                    child.finish().await
                });
                ready.await.unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            _ => {
                let task = tokio::spawn(async move {
                    let _child = child;
                    panic!("controlled caller panic")
                });
                assert!(task.await.unwrap_err().is_panic());
            }
        }
        let completion = process::retired(&mut retired).await;
        assert!(
            completion.cancelled,
            "termination must be caused by caller retirement"
        );
        let output = completion.output;
        assert!(
            output.stderr.is_empty(),
            "termination must precede a protocol timeout: {output:?}"
        );
        assert!(!output.status.success(), "{mode}: {output:?}");
        assert_rebind(source);
    }
}

#[tokio::test]
async fn server_owner_stops_after_caller_panic_or_cancellation() {
    for panic in [false, true] {
        let server = Server::start(Some(72.5)).await;
        let address = server.address;
        let mut stopped = server.done.clone();
        let (started, ready) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _server = server;
            let _ = started.send(());
            assert!(!panic, "controlled fixture caller panic");
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        if !panic {
            task.abort();
        }
        let result = task.await.unwrap_err();
        assert!(if panic {
            result.is_panic()
        } else {
            result.is_cancelled()
        });
        timeout(WAIT, stopped.wait_for(Option::is_some))
            .await
            .unwrap()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .unwrap();
        assert_rebind(address);
    }
}

#[tokio::test]
async fn an_existing_nonexecutable_override_fails_without_fallback() {
    let source = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/first_read.rs"));
    let selected = process::select_binary(Some(source.as_os_str())).unwrap();
    assert_eq!(selected, source.canonicalize().unwrap());
    assert!(Process::spawn(&selected, &[]).is_err());
}

#[test]
fn child_and_server_retire_even_when_the_callers_runtime_shuts_down() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut child_done, child_address, mut server_done, server_address) =
        runtime.block_on(async {
            let peer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let child = Process::spawn(
                &process::binary(),
                &[&peer.local_addr().unwrap().to_string()],
            )
            .unwrap();
            let child_done = child.retirement();
            let server = Server::start(Some(72.5)).await;
            let server_address = server.address;
            let server_done = server.done.clone();
            let mut bytes = [0; 2048];
            let (_, child_address) = timeout(WAIT, peer.recv_from(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            let (started, ready) = oneshot::channel();
            tokio::spawn(async move {
                let _child = child;
                let _server = server;
                let _ = started.send(());
                std::future::pending::<()>().await;
            });
            ready.await.unwrap();
            (child_done, child_address, server_done, server_address)
        });
    drop(runtime);
    let observer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    observer.block_on(async {
        let completion = process::retired(&mut child_done).await;
        assert!(completion.cancelled);
        assert!(!completion.output.status.success());
        timeout(WAIT, server_done.wait_for(Option::is_some))
            .await
            .unwrap()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .unwrap();
    });
    assert_rebind(child_address);
    assert_rebind(server_address);
}
