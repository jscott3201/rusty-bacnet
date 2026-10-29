//! The MS/TP endpoint's link options (#1663): `mstp_extended_frames` and
//! the echo keywords, with the rules `BACnetClient` and `BACnetServer` have
//! (#1653). The serial port is injected, so nothing opens a device.
use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::mstp::codec::{self, FrameType};
use bacnet_transport::mstp::{LoopbackSerial, SerialPort};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::lifecycle_tests::TestSerial;
use super::*;
use crate::mstp_py::{ECHO_GRACE_WITHOUT_ECHO_ERROR, ZERO_ECHO_GRACE_ERROR};

const SESSION_MAC: u8 = 3;
const PEER_MAC: u8 = 7;

/// An endpoint with the given `max_apdu` and link options, and every other
/// argument at its default.
fn endpoint(
    max_apdu: u16,
    extended_frames: bool,
    echo: bool,
    echo_grace_ms: Option<u32>,
) -> PyResult<PyMstpEndpoint> {
    timed_endpoint(
        MstpTiming::default(),
        max_apdu,
        extended_frames,
        echo,
        echo_grace_ms,
    )
}

/// As [`endpoint`], with the timing keywords `timing` gives.
fn timed_endpoint(
    timing: MstpTiming,
    max_apdu: u16,
    extended_frames: bool,
    echo: bool,
    echo_grace_ms: Option<u32>,
) -> PyResult<PyMstpEndpoint> {
    PyMstpEndpoint::new(
        8010,
        "injected-serial",
        "Link options",
        555,
        38400,
        SESSION_MAC,
        127,
        1,
        max_apdu,
        None,
        None,
        None,
        16,
        6000,
        0,
        256,
        0,
        extended_frames,
        echo,
        echo_grace_ms,
        timing.transmit_latency_pad_ms,
        timing.usage_timeout_ms,
        timing.reply_timeout_ms,
        None,
        None,
        None,
        None,
    )
}

/// The timing keywords reach the link configuration the session is built
/// with; left out they keep `MstpConfig::default()`'s. A pad over 15 ms or
/// a timeout outside Clause 9.5.3's ranges is a ValueError naming the
/// field, at construction.
#[test]
fn timing_keywords_reach_the_link_and_are_checked() {
    Python::initialize();
    let defaults = MstpConfig::default();
    let link = endpoint(480, false, false, None).unwrap().config.link();
    assert_eq!(
        (
            link.transmit_latency_pad,
            link.usage_timeout,
            link.reply_timeout
        ),
        (
            defaults.transmit_latency_pad,
            defaults.usage_timeout,
            defaults.reply_timeout
        )
    );
    let timing = MstpTiming {
        transmit_latency_pad_ms: Some(10),
        usage_timeout_ms: Some(30),
        reply_timeout_ms: Some(290),
    };
    let link = timed_endpoint(timing, 480, false, false, None)
        .unwrap()
        .config
        .link();
    assert_eq!(link.transmit_latency_pad, Duration::from_millis(10));
    assert_eq!(link.usage_timeout, Duration::from_millis(30));
    assert_eq!(link.reply_timeout, Duration::from_millis(290));
    // The session's builder hands the transport exactly that link.
    let (serial, _peer) = LoopbackSerial::pair();
    let transport = link_builder(serial, &link).build_transport().unwrap();
    assert_eq!(transport.config(), &link);
    for (timing, field) in [
        (
            MstpTiming {
                transmit_latency_pad_ms: Some(16),
                ..MstpTiming::default()
            },
            "transmit_latency_pad",
        ),
        (
            MstpTiming {
                usage_timeout_ms: Some(36),
                ..MstpTiming::default()
            },
            "usage_timeout",
        ),
        (
            MstpTiming {
                reply_timeout_ms: Some(254),
                ..MstpTiming::default()
            },
            "reply_timeout",
        ),
    ] {
        let message = value_error(timed_endpoint(timing, 480, false, false, None));
        assert!(message.contains(field), "{message}");
    }
}

fn value_error(result: PyResult<PyMstpEndpoint>) -> String {
    let Err(error) = result else {
        panic!("expected a ValueError");
    };
    Python::attach(|py| {
        assert!(error.is_instance_of::<PyValueError>(py));
        error.value(py).to_string()
    })
}

/// By default the link runs standard frames, so `max_apdu` is held to 480;
/// `mstp_extended_frames=True` lifts the bound to 1476.
#[test]
fn extended_frames_lift_the_apdu_bound_to_1476() {
    Python::initialize();
    let default = endpoint(480, false, false, None).unwrap();
    assert!(!default.config.extended_frames);
    assert_eq!(default.config.echo, Echo::None);

    let message = value_error(endpoint(1476, false, false, None));
    assert!(message.contains("bound 480"), "{message}");
    assert!(message.contains("mstp_extended_frames=True"), "{message}");

    let extended = endpoint(1476, true, false, None).unwrap();
    assert!(extended.config.extended_frames);
    assert_eq!(extended.config.identity.max_apdu_length(), 1476);
    assert_eq!(mstp_max_apdu(false), 480);
    assert_eq!(mstp_max_apdu(true), 1476);
}

/// The echo keywords follow the client's and server's rules: off by
/// default, the transport's grace with `mstp_echo` alone, a grace of its
/// own; a zero grace, or any grace without the echo, is a ValueError.
#[test]
fn echo_keywords_follow_the_client_and_server_rules() {
    Python::initialize();
    for (echo, grace, expected) in [
        (false, None, Echo::None),
        (true, None, Echo::full()),
        (
            true,
            Some(25),
            Echo::Full {
                grace: Duration::from_millis(25),
            },
        ),
    ] {
        let built = endpoint(480, false, echo, grace).unwrap();
        assert_eq!(built.config.echo, expected, "{echo} {grace:?}");
    }
    for (echo, grace, expected) in [
        (true, Some(0), ZERO_ECHO_GRACE_ERROR),
        (false, Some(0), ECHO_GRACE_WITHOUT_ECHO_ERROR),
        (false, Some(25), ECHO_GRACE_WITHOUT_ECHO_ERROR),
    ] {
        assert_eq!(value_error(endpoint(480, false, echo, grace)), expected);
    }
}

/// The port the endpoint opens declares the echo the keywords ask for.
#[test]
fn the_opened_port_declares_the_echo() {
    Python::initialize();
    for (echo, grace, expected) in [
        (false, None, Echo::None),
        (true, None, Echo::full()),
        (
            true,
            Some(60),
            Echo::Full {
                grace: Duration::from_millis(60),
            },
        ),
    ] {
        let mut built = endpoint(480, false, echo, grace).unwrap();
        built.config.serial_opener = Some(Arc::new(|_| {
            let (serial, _peer) = LoopbackSerial::pair();
            Ok(TestSerial::Peer(serial))
        }));
        let serial = built.config.open_serial().unwrap();
        assert_eq!(serial.echo(), expected, "{echo} {grace:?}");
    }
}

/// A ReadProperty of AI 1's Object_Name from a requester that accepts
/// 1476-octet APDUs, in a Data Expecting Reply frame from the peer.
fn read_name_frame(invoke_id: u8) -> Vec<u8> {
    let mut service = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
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
            max_apdu_length: 1476,
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
    let mut wire = Vec::new();
    codec::Frame::new(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        SESSION_MAC,
        PEER_MAC,
        &npdu,
    )
    .encode_to_sink(&mut wire)
    .unwrap();
    wire
}

/// Reads the session's frames until its answer to the peer comes, and
/// returns that frame's type and NPDU. An extended frame's data is
/// COBS-decoded in place.
async fn answer_to_peer(peer: &LoopbackSerial) -> (FrameType, Vec<u8>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 2048];
    loop {
        let read = tokio::time::timeout_at(deadline, peer.read(&mut buf))
            .await
            .expect("the session answered")
            .unwrap();
        let (frame, _) = codec::decode_frame_in_place(&mut buf[..read]).unwrap();
        let data_frame = matches!(
            frame.header.frame_type,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
        );
        if frame.header.destination == PEER_MAC && data_frame {
            return (frame.header.frame_type, frame.data.to_vec());
        }
    }
}

/// With `mstp_extended_frames=True` and `max_apdu=1476`, the Python
/// endpoint answers a read whose ComplexAck is far over 480 octets in an
/// extended frame, through the same wrapper and session a real port uses.
#[test]
fn an_extended_endpoint_answers_a_long_read_in_an_extended_frame() {
    Python::initialize();
    let name: String = (0..1_400)
        .map(|index| char::from(b'a' + (index % 26) as u8))
        .collect();
    let peer = Arc::new(std::sync::Mutex::new(None));
    let mut built = endpoint(1476, true, false, None).unwrap();
    built.config.serial_opener = Some(Arc::new({
        let peer = peer.clone();
        move |_| {
            let (serial, other) = LoopbackSerial::pair();
            *peer.lock().unwrap() = Some(other);
            Ok(TestSerial::Peer(serial))
        }
    }));
    built
        .push_pending(PendingObject::AnalogInput {
            instance: 1,
            name: name.clone(),
            units: 62,
            present_value: 0.0,
        })
        .unwrap();
    let built = Python::attach(|py| Py::new(py, built).unwrap());
    let run = |script: &std::ffi::CStr| {
        Python::attach(|py| {
            let globals = PyDict::new(py);
            globals.set_item("endpoint", built.clone_ref(py)).unwrap();
            py.run(script, Some(&globals), None).unwrap();
        });
    };
    run(pyo3::ffi::c_str!(
        "import asyncio\nasync def go():\n    await endpoint.start()\nasyncio.run(go())\n"
    ));
    let peer = peer.lock().unwrap().take().expect("serial opened at start");
    let (frame_type, npdu) = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            peer.write(&read_name_frame(9)).await.unwrap();
            answer_to_peer(&peer).await
        });
    run(pyo3::ffi::c_str!(
        "import asyncio\nasync def go():\n    await endpoint.close()\nasyncio.run(go())\n"
    ));
    assert_eq!(
        frame_type,
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    );
    let npdu = decode_npdu(npdu.into()).unwrap();
    assert!(npdu.payload.len() > 480, "{}", npdu.payload.len());
    let Apdu::ComplexAck(ack) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected a ComplexAck");
    };
    assert_eq!(ack.invoke_id, 9);
    let read = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    assert_eq!(read.property_identifier, PropertyIdentifier::OBJECT_NAME);
    assert!(read.property_value.len() > 1_400);
}
