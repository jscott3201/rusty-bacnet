//! The default admission policy at its edge (#1979 S2, experiment E3): 64
//! confirmed slots less the recovery reserve of 4 leave 60 ordinary slots,
//! so the 61st confirmed request that arrives while 60 handlers are live is
//! refused with an Abort. Every slot here stays held because the fixture's
//! transport parks each reply in its send until the test releases it: a
//! slot lasts until the reply is sent.
use super::*;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::primitives::ObjectIdentifier;

/// A ReadProperty of the fixture Device's Object_Name with `invoke_id`.
fn read_device_name(invoke_id: u8) -> Apdu {
    let device = bacnet_objects::DeviceIdentity::default().instance();
    let mut service_request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut service_request);
    let Apdu::ConfirmedRequest(mut req) = confirmed(false) else {
        unreachable!()
    };
    req.invoke_id = invoke_id;
    req.service_request = service_request.freeze();
    Apdu::ConfirmedRequest(req)
}

/// Dispatches `apdu` as if it came straight from the station `mac`, so each
/// request below is its own peer and the per-peer limit of 16 never binds.
async fn dispatch_from(server: &BACnetServer<TestTransport>, mac: u8, apdu: Apdu) {
    BACnetServer::dispatch(
        &server.test_dispatch_context(),
        &[mac],
        apdu,
        bacnet_network::layer::ReceivedApdu {
            router_response: None,
            direct_response: None,
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(&[mac]),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx: None,
        },
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn default_policy_answers_60_and_aborts_the_61st_confirmed_request() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "default admission",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy::default(),
            ..ServerConfig::default()
        },
    )
    .await;
    // All 61 are admitted or refused here, before any handler task runs:
    // dispatch only registers a handler, and this test hasn't yielded yet.
    for id in 0..61u8 {
        dispatch_from(&server, 10 + id, read_device_name(id)).await;
    }
    assert!(
        started.try_recv().is_err(),
        "no handler reached its reply before all 61 were dispatched"
    );
    let counters = server.request_admission_counters();
    assert_eq!(counters.confirmed_admitted_total, 60);
    assert_eq!(counters.confirmed_active, 60);
    assert_eq!(counters.confirmed_global_overloaded_total, 1);
    assert_eq!(counters.confirmed_peer_overloaded_total, 0);
    assert_eq!(counters.abort_admitted_total, 1);
    assert_eq!(counters.confirmed_fallback_dropped_total, 0);
    assert_eq!(server.request_admission_confirmed_peak(), 60);

    // 60 replies and the Abort reach the transport and wait there.
    let mut parked = Vec::new();
    for _ in 0..61 {
        parked.push(observed(&mut started).await);
    }
    {
        let frames = held_sends(&server).frames.lock().unwrap();
        assert_eq!(frames.len(), 61);
        let aborts: Vec<_> = frames
            .iter()
            .filter_map(|frame| match frame {
                Apdu::Abort(abort) => Some(abort),
                _ => None,
            })
            .collect();
        assert_eq!(aborts.len(), 1, "exactly one Abort: {frames:?}");
        assert!(aborts[0].sent_by_server);
        assert_eq!(aborts[0].invoke_id, 60, "the request past the 60th");
        assert_eq!(aborts[0].abort_reason, AbortReason::OUT_OF_RESOURCES);
        let mut answered: Vec<u8> = frames
            .iter()
            .filter_map(|frame| match frame {
                Apdu::ComplexAck(ack) => Some(ack.invoke_id),
                _ => None,
            })
            .collect();
        answered.sort_unstable();
        assert_eq!(answered, (0..60).collect::<Vec<u8>>());
    }

    // Releasing the sends ends the handlers; the slots come back, the peak stays.
    held_sends(&server).release.notify_waiters();
    for done in parked {
        tokio::time::timeout(Duration::from_secs(2), done)
            .await
            .expect("a parked send finished")
            .expect("its guard sent");
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.request_admission_counters().confirmed_active != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("all 60 handlers released their slots");
    assert_eq!(server.request_admission_confirmed_peak(), 60);
    server.stop().await.unwrap();
}
