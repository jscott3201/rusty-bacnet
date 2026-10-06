//! Private injection: exercises the actual MS/TP Python wrapper and session.
//! Darwin PTYs reject the serial backend's ioctl; this is no hardware claim.
use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::mstp::{LoopbackSerial, SerialPort};
use bacnet_transport::mstp_frame::{decode_frame, encode_frame, FrameType, MstpFrame};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub(super) type SerialOpener = Arc<dyn Fn(&SerialConfig) -> PyResult<TestSerial> + Send + Sync>;

pub(super) enum TestSerial {
    Real(TokioSerialPort),
    Injected(Box<TrackedSerial>),
    /// The test drives the other end as a raw-frame MS/TP peer.
    Peer(LoopbackSerial),
}
pub(super) struct TrackedSerial {
    serial: LoopbackSerial,
    _peer: LoopbackSerial,
    live: Arc<AtomicUsize>,
}
impl Drop for TrackedSerial {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}
impl SerialPort for TestSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        match self {
            Self::Real(port) => port.write(data).await,
            Self::Injected(port) => port.serial.write(data).await,
            Self::Peer(port) => port.write(data).await,
        }
    }
    async fn drain(&self) -> Result<(), Error> {
        match self {
            Self::Real(port) => port.drain().await,
            Self::Injected(port) => port.serial.drain().await,
            Self::Peer(port) => port.drain().await,
        }
    }
    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        match self {
            Self::Real(port) => port.read(buf).await,
            Self::Injected(port) => port.serial.read(buf).await,
            Self::Peer(port) => port.read(buf).await,
        }
    }
}

#[test]
fn mstp_python_wrapper_owns_one_serial_and_returns_none() {
    Python::initialize();
    let opens = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(AtomicUsize::new(0));
    let mut endpoint = PyMstpEndpoint::new(
        8002,
        "injected-serial",
        "Lifecycle",
        555,
        38400,
        3,
        127,
        1,
        480,
        None,
        None,
        None,
        16,
        6000,
        0,
        256,
        0,
    )
    .unwrap();
    endpoint.config.serial_opener = Some(Arc::new({
        let opens = opens.clone();
        let live = live.clone();
        move |_| {
            opens.fetch_add(1, Ordering::SeqCst);
            live.fetch_add(1, Ordering::SeqCst);
            let (serial, peer) = LoopbackSerial::pair();
            Ok(TestSerial::Injected(Box::new(TrackedSerial {
                serial,
                _peer: peer,
                live: live.clone(),
            })))
        }
    }));
    Python::attach(|py| {
        let globals = PyDict::new(py);
        globals
            .set_item("endpoint", Py::new(py, endpoint).unwrap())
            .unwrap();
        py.run(pyo3::ffi::c_str!(r#"
import asyncio
async def exercise():
    results = await asyncio.gather(endpoint.start(), endpoint.start(), return_exceptions=True)
    assert sum(result is None for result in results) == 1, results
    assert sum(isinstance(result, Exception) for result in results) == 1, results
    assert 'endpoint already started' in str(next(result for result in results if result is not None))
    assert await endpoint.__aenter__() is endpoint
    assert (await endpoint.status())['is_running']
    assert await endpoint.broadcast_i_am() is None
    assert await endpoint.__aexit__(None, None, None) is None
    assert await endpoint.close() is None
    endpoint.add_analog_input(instance=1, name='Restart')
    assert await endpoint.start() is None
    assert await endpoint.__aenter__() is endpoint
    assert await endpoint.close() is None
    assert await endpoint.__aexit__(None, None, None) is None
asyncio.run(exercise())
"#),Some(&globals),None).unwrap();
    });
    assert_eq!(
        opens.load(Ordering::SeqCst),
        2,
        "one open per admitted session, including restart"
    );
    assert_eq!(
        live.load(Ordering::SeqCst),
        0,
        "awaited close releases serial owner"
    );
}

const SESSION_MAC: u8 = 3;
const PEER_MAC: u8 = 7;

/// ReadProperty of Group `instance`'s Present_Value, as the NPDU of a
/// DataExpectingReply frame from the peer.
fn group_read_frame(invoke_id: u8, instance: u32) -> MstpFrame {
    let mut service = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::GROUP, instance).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut service);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: npdu.freeze(),
    }
}

async fn peer_send(peer: &LoopbackSerial, frame: &MstpFrame) {
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, frame).unwrap();
    peer.write(&wire).await.unwrap();
}

/// Sends one Group read and returns the APDU the session answers it with.
/// Every session write is one frame. A ReplyPostponed gets a token back so the
/// session can send the reply; other link traffic is ignored, and the request
/// is repeated if a second passes without an answer to it.
async fn read_group(peer: &LoopbackSerial, invoke_id: u8, instance: u32) -> Apdu {
    let request = group_read_frame(invoke_id, instance);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut buf = [0u8; 2048];
    loop {
        peer_send(peer, &request).await;
        let resend = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        while let Ok(read) =
            tokio::time::timeout_at(resend.min(deadline), peer.read(&mut buf)).await
        {
            let (frame, _) = decode_frame(&buf[..read.unwrap()]).unwrap();
            if frame.destination != PEER_MAC {
                continue;
            }
            match frame.frame_type {
                FrameType::ReplyPostponed => {
                    let token = MstpFrame {
                        frame_type: FrameType::Token,
                        destination: SESSION_MAC,
                        source: PEER_MAC,
                        data: Bytes::new(),
                    };
                    peer_send(peer, &token).await;
                }
                FrameType::BACnetDataNotExpectingReply => {
                    let npdu = decode_npdu(frame.data).unwrap();
                    let apdu = decode_apdu(npdu.payload).unwrap();
                    let answered = match &apdu {
                        Apdu::Abort(abort) => abort.invoke_id,
                        Apdu::ComplexAck(ack) => ack.invoke_id,
                        other => panic!("unexpected reply {other:?}"),
                    };
                    if answered == invoke_id {
                        return apdu;
                    }
                }
                _ => {}
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no answer to Group {instance}"
        );
    }
}

/// The wrapper hands `read_work_limit` to the MS/TP session (#1250): with a
/// limit of 2, Group 1's three rows are aborted with OUT_OF_RESOURCES and
/// Group 2's two rows are served.
#[test]
fn mstp_python_wrapper_applies_its_read_work_limit() {
    Python::initialize();
    let peer = Arc::new(std::sync::Mutex::new(None));
    let mut endpoint = PyMstpEndpoint::new(
        8003,
        "injected-serial",
        "Read work limit",
        555,
        38400,
        SESSION_MAC,
        127,
        1,
        480,
        None,
        None,
        None,
        16,
        6000,
        0,
        2,
        0,
    )
    .unwrap();
    assert_eq!(endpoint.config.read_work_limit, 2);
    endpoint.config.serial_opener = Some(Arc::new({
        let peer = peer.clone();
        move |_| {
            let (serial, other) = LoopbackSerial::pair();
            *peer.lock().unwrap() = Some(other);
            Ok(TestSerial::Peer(serial))
        }
    }));
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 8003).unwrap();
    for (instance, properties) in [
        (
            1,
            &[
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::OBJECT_TYPE,
            ][..],
        ),
        (2, &[PropertyIdentifier::OBJECT_NAME][..]),
    ] {
        endpoint
            .push_pending(PendingObject::Group {
                instance,
                name: format!("GRP-{instance}"),
                members: vec![ReadAccessSpecification {
                    object_identifier: device,
                    list_of_property_references: properties
                        .iter()
                        .map(|&property_identifier| PropertyReference {
                            property_identifier,
                            property_array_index: None,
                        })
                        .collect(),
                }],
            })
            .unwrap();
    }
    let endpoint = Python::attach(|py| Py::new(py, endpoint).unwrap());
    let run = |script: &std::ffi::CStr| {
        Python::attach(|py| {
            let globals = PyDict::new(py);
            globals
                .set_item("endpoint", endpoint.clone_ref(py))
                .unwrap();
            py.run(script, Some(&globals), None).unwrap();
        });
    };
    run(pyo3::ffi::c_str!(
        "import asyncio\nasync def go():\n    await endpoint.start()\nasyncio.run(go())\n"
    ));
    let peer = peer.lock().unwrap().take().expect("serial opened at start");
    let replies = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async { [read_group(&peer, 1, 1).await, read_group(&peer, 2, 2).await] });
    run(pyo3::ffi::c_str!(
        "import asyncio\nasync def go():\n    await endpoint.close()\nasyncio.run(go())\n"
    ));
    assert!(
        matches!(
            &replies[0],
            Apdu::Abort(abort) if abort.abort_reason == AbortReason::OUT_OF_RESOURCES
        ),
        "{replies:?}"
    );
    assert!(matches!(&replies[1], Apdu::ComplexAck(_)), "{replies:?}");
}
