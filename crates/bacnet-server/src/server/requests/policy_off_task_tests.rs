//! DeviceCommunicationControl, LifeSafetyOperation and
//! UnconfirmedAuditNotification ask their `ServerPolicy` method off the
//! request task (#2032): `stop()`, which joins every admitted request, ends
//! while such a policy is still blocked.

use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::AuditOperation;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::sync::mpsc;

/// A blocking wait the test releases; `entered` fires once it is blocked.
struct Gate {
    block: Arc<dyn Fn() + Send + Sync>,
    entered: mpsc::UnboundedReceiver<()>,
    release: std_mpsc::Sender<()>,
}

fn gate() -> Gate {
    let (entered_tx, entered) = mpsc::unbounded_channel();
    let (release, release_rx) = std_mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    Gate {
        block: Arc::new(move || {
            entered_tx.send(()).unwrap();
            // Holds this thread, as a Python callable waiting on another
            // thread would.
            release_rx.lock().unwrap().recv().unwrap();
        }),
        entered,
        release,
    }
}

/// Feed `apdu` to the server as one NPDU from MAC 1.
async fn send(inbound: &mpsc::Sender<ReceivedNpdu>, apdu: Apdu) {
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: encoded.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    inbound
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(&[1]),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

fn confirmed(service_choice: ConfirmedServiceChoice, service_request: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request,
    })
}

fn life_safety() -> Apdu {
    let mut data = BytesMut::new();
    bacnet_services::life_safety::LifeSafetyOperationRequest {
        requesting_process_identifier: 7,
        requesting_source: "operator".into(),
        request: bacnet_types::enums::LifeSafetyOperation::SILENCE,
        object_identifier: None,
    }
    .encode(&mut data)
    .unwrap();
    confirmed(ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, data.freeze())
}

fn dcc() -> Apdu {
    // DISABLE_INITIATION with no duration and no password.
    confirmed(
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        Bytes::from_static(b"\x19\x02"),
    )
}

fn unconfirmed_audit() -> Apdu {
    let notification = crate::server::audit_notification_tests::notification(AuditOperation::WRITE);
    Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
        service_request: crate::server::audit_notification_tests::request_bytes(vec![notification]),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_while_a_dcc_life_safety_or_audit_policy_is_blocked() {
    for service in ["dcc", "life_safety", "unconfirmed_audit"] {
        let mut gate = gate();
        let block = Arc::clone(&gate.block);
        let policy = match service {
            "dcc" => crate::FnPolicy::new().device_communication_control(move |_| {
                block();
                true
            }),
            "life_safety" => crate::FnPolicy::new().life_safety_operation(move |_| {
                block();
                true
            }),
            _ => crate::FnPolicy::new().unconfirmed_audit_notification(move |_| {
                block();
                true
            }),
        };
        let (transport, inbound) = TestTransport::inbound(4);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(100, 0).unwrap(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let mut server = BACnetServer::generic_builder()
            .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
            .transport(transport)
            .database(db)
            .enable_event_enrollment(false)
            .dcc_policy(DccPolicy::LegacyPermissive)
            .audit_notification_sink(ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap())
            .policy(policy)
            .build()
            .await
            .unwrap();
        let request = match service {
            "dcc" => dcc(),
            "life_safety" => life_safety(),
            _ => unconfirmed_audit(),
        };
        send(&inbound, request).await;
        tokio::time::timeout(Duration::from_secs(10), gate.entered.recv())
            .await
            .unwrap_or_else(|_| panic!("{service}: the policy was asked"))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), server.stop())
            .await
            .unwrap_or_else(|_| panic!("{service}: stop ends while the policy is blocked"))
            .unwrap();
        // The abandoned call finishes on its own thread and is ignored.
        gate.release.send(()).unwrap();
        if service == "dcc" {
            assert_eq!(server.comm_state(), DccState::Enable, "{service}");
        }
    }
}
