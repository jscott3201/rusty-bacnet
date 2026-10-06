use super::*;
use crate::mutation::{MutationAuthorizationContext, MutationTarget};

async fn complete_replacement(lso: bool, same_leaf: bool) {
    let ca = TestCa::new();
    let a_tls = ca.tls("a");
    let b_tls = if same_leaf {
        a_tls.clone()
    } else {
        ca.tls("b")
    };
    let allowed = Arc::new(StdMutex::new(None::<DirectScIdentity>));
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let allowed_mutation = allowed.clone();
    let observed_mutation = observed.clone();
    let allowed_lso = allowed.clone();
    let observed_lso = observed.clone();
    let config = ServerConfig {
        mutation_authorizer: Some(Arc::new(move |context| {
            let identity = context.direct_sc_identity().unwrap();
            observed_mutation
                .lock()
                .unwrap()
                .push((identity, format!("{context:?}")));
            Some(identity) == *allowed_mutation.lock().unwrap()
        })),
        life_safety_operation_authorizer: Some(Arc::new(move |context| {
            let identity = context.direct_sc_identity().unwrap();
            observed_lso
                .lock()
                .unwrap()
                .push((identity, format!("{context:?}")));
            Some(identity) == *allowed_lso.lock().unwrap()
        })),
        ..Default::default()
    };
    let executions = Arc::new(AtomicUsize::new(0));
    let mut f = Fixture::new(&ca, config, database(executions.clone())).await;
    let mut a = f.peer(a_tls).await;
    let request = request(lso, 42);
    let first = f.capture(&mut a, &request).await;
    let a_identity = first.provenance.direct_sc_identity().unwrap();
    *allowed.lock().unwrap() = Some(a_identity);
    let original_wire = first.npdu.clone();
    let same_socket_retry = f.capture(&mut a, &request).await;
    assert_eq!(first.provenance, same_socket_retry.provenance);

    // A is admitted while the database gate keeps its execution pending.
    let db = f.server.db.clone();
    let held = db.write().await;
    f.feed(first).await;
    f.feed(same_socket_retry).await;
    f.dispatch_barrier(&mut a, 240).await;
    f.active(1).await;
    assert_eq!(
        f.server.request_tasks.counters().confirmed_admitted_total,
        2
    );

    // Replacement changes the principal/incarnation, even though every claimed
    // address, invoke ID and encoded request byte is unchanged.
    let mut b = f.peer(b_tls.clone()).await;
    let contender = f.capture(&mut b, &request).await;
    let b_identity = contender.provenance.direct_sc_identity().unwrap();
    assert_ne!(a_identity.incarnation(), b_identity.incarnation());
    assert_eq!(
        a_identity.leaf_sha256() == b_identity.leaf_sha256(),
        same_leaf
    );
    assert_eq!(original_wire, contender.npdu);
    f.feed(contender).await;
    let mut outcomes = f.barrier_responses(&mut b, 241).await;
    // B's fresh work is never suppressed by A's pending duplicate entry.
    assert_eq!(
        f.server.request_tasks.counters().confirmed_admitted_total,
        4
    );
    drop(held);
    while outcomes.is_empty() {
        outcomes.push(f.response().await);
    }
    assert_eq!(
        outcomes
            .iter()
            .filter(|a| matches!(a, Apdu::SimpleAck(_)))
            .count(),
        0 // retired A finishes, but its ACK cannot be delivered to B
    );
    assert_eq!(outcomes.iter().filter(|a| denied(a)).count(), 1);
    f.active(0).await;
    assert!(f.responses.try_recv().is_err());
    let identities: Vec<_> = observed.lock().unwrap().iter().map(|(id, _)| *id).collect();
    assert_eq!(identities.len(), 2);
    assert!(identities.contains(&a_identity) && identities.contains(&b_identity));
    assert_eq!(executions.load(Ordering::Acquire), usize::from(lso));
    if !lso {
        assert_eq!(
            db.read()
                .await
                .get(&csv_oid())
                .unwrap()
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::CharacterString("A value".into())
        );
    }

    // After terminal issuance, ordinary WP is a fresh same-socket operation and
    // reauthorizes B (still denied). LSO retains its distinct completed replay.
    let retry = f.capture(&mut b, &request).await;
    f.feed(retry).await;
    assert!(denied(&f.response().await));
    f.dispatch_barrier(&mut b, 242).await;
    f.active(0).await;
    let after_retry = if lso { 2 } else { 3 };
    assert_eq!(observed.lock().unwrap().len(), after_retry);
    if !lso {
        assert_eq!(observed.lock().unwrap()[2].0, b_identity);
    }

    // Same certificate reconnect has a new incarnation and must reach policy,
    // including while LSO retains that certificate's old completed result.
    let mut c = f.peer(b_tls).await;
    let fresh = f.capture(&mut c, &request).await;
    let c_identity = fresh.provenance.direct_sc_identity().unwrap();
    assert_eq!(c_identity.leaf_sha256(), b_identity.leaf_sha256());
    assert_ne!(c_identity.incarnation(), b_identity.incarnation());
    assert_eq!(original_wire, fresh.npdu);
    f.feed(fresh).await;
    assert!(denied(&f.response().await));
    f.active(0).await;
    let observed = observed.lock().unwrap().clone();
    assert_eq!(observed.len(), after_retry + 1);
    assert_eq!(observed[after_retry].0, c_identity);
    for (_, debug) in observed {
        assert!(!debug.contains("private operator claim"));
        assert!(!debug.contains("A value"));
        assert!(!debug.contains(&format!("{:?}", a_identity.leaf_sha256())));
    }
    f.stop().await;
}

#[tokio::test]
async fn direct_principal_wp_replacement_pending_and_completed() {
    complete_replacement(false, false).await;
}
#[tokio::test]
async fn direct_principal_wp_same_leaf_reconnect_pending_and_completed() {
    complete_replacement(false, true).await;
}
#[tokio::test]
async fn direct_principal_lso_replacement_pending_and_completed() {
    complete_replacement(true, false).await;
}
#[tokio::test]
async fn direct_principal_lso_same_leaf_reconnect_pending_and_completed() {
    complete_replacement(true, true).await;
}

#[tokio::test]
async fn direct_principal_wpm_elements_keep_admitted_snapshot_and_order() {
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
    let ca = TestCa::new();
    let observed = Arc::new(StdMutex::new(Vec::<MutationAuthorizationContext>::new()));
    let policy = observed.clone();
    let config = ServerConfig {
        mutation_authorizer: Some(Arc::new(move |context| {
            let mut seen = policy.lock().unwrap();
            seen.push(context.clone());
            seen.len() == 1 // commit the first element, deny the second, never see the third
        })),
        ..Default::default()
    };
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;
    let mut a = f.peer(ca.tls("a")).await;
    let properties = ["first", "denied", "unattempted"].map(|text| {
        let mut value = BytesMut::new();
        encode_property_value(&mut value, &PropertyValue::CharacterString(text.into())).unwrap();
        BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: value.to_vec(),
            priority: None,
        }
    });
    let mut wire = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: csv_oid(),
            list_of_properties: properties.to_vec(),
        }],
    }
    .encode(&mut wire)
    .unwrap();
    let request = confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        44,
        wire.freeze(),
    );
    let admitted = f.capture(&mut a, &request).await;
    let a_identity = admitted.provenance.direct_sc_identity().unwrap();
    let db = f.server.db.clone();
    let held = db.write().await;
    f.feed(admitted).await;
    f.dispatch_barrier(&mut a, 240).await;
    f.active(1).await;
    let mut b = f.peer(ca.tls("b")).await;
    let b_envelope = f.capture(&mut b, &request).await;
    assert_ne!(
        a_identity,
        b_envelope.provenance.direct_sc_identity().unwrap()
    );
    drop(held);
    f.dispatch_barrier(&mut b, 241).await;
    f.active(0).await;
    assert!(f.responses.try_recv().is_err());
    let seen = observed.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    for (index, context) in seen.iter().enumerate() {
        assert_eq!(context.direct_sc_identity(), Some(a_identity));
        let MutationTarget::WritePropertyMultiple(attempt) = &context.target else {
            panic!("wrong target");
        };
        assert_eq!(attempt.value, properties[index].value);
    }
    assert_eq!(
        db.read()
            .await
            .get(&csv_oid())
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString("first".into())
    );
    f.stop().await;
}

/// A ReinitializeDevice handler sees the verified direct-SC identity of the
/// connection that admitted the request, next to its claimed sources.
#[tokio::test]
async fn direct_principal_reinitialize_context_carries_the_admitted_identity() {
    use crate::server::ReinitializeContext;
    use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
    use bacnet_types::enums::ReinitializedState;
    let ca = TestCa::new();
    let observed = Arc::new(StdMutex::new(Vec::<ReinitializeContext>::new()));
    let recorded = observed.clone();
    let config = ServerConfig {
        on_reinitialize: Some(Arc::new(
            move |context: &ReinitializeContext, _: &mut ObjectDatabase| {
                recorded.lock().unwrap().push(context.clone());
                Ok(())
            },
        )),
        ..Default::default()
    };
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;
    let mut a = f.peer(ca.tls("a")).await;
    let mut wire = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: None,
    }
    .encode(&mut wire)
    .unwrap();
    let request = confirmed(
        ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        45,
        wire.freeze(),
    );
    let admitted = f.capture(&mut a, &request).await;
    let identity = admitted.provenance.direct_sc_identity().unwrap();
    f.feed(admitted).await;
    let response = f.response().await;
    assert!(
        matches!(response, Apdu::SimpleAck(ref ack) if ack.invoke_id == 45),
        "got {response:?}"
    );
    let context = observed.lock().unwrap().pop().unwrap();
    assert_eq!(context.direct_sc_identity(), Some(identity));
    assert_eq!(context.source_mac.as_ref(), PEER_MAC);
    assert_eq!(
        context.source_network,
        Some(NpduAddress {
            network: 123,
            mac_address: MacAddr::from_slice(&[3]),
        })
    );
    f.stop().await;
}

/// The endpoint responder passes the received provenance through to the
/// handler: a request admitted over direct BACnet/SC reaches it with that
/// connection's verified identity, not a default provenance.
#[tokio::test]
async fn direct_principal_endpoint_reinitialize_context_keeps_the_provenance() {
    use crate::server::requests::endpoint_responder::EndpointResponder;
    use crate::server::ReinitializeContext;
    use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
    use bacnet_network::layer::ReceivedApdu;
    use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
    use bacnet_transport::loopback::LoopbackTransport;
    use bacnet_types::enums::ReinitializedState;
    let ca = TestCa::new();
    let mut f = Fixture::new(&ca, ServerConfig::default(), ObjectDatabase::new()).await;
    let mut a = f.peer(ca.tls("a")).await;
    let mut wire = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::START_BACKUP,
        password: None,
    }
    .encode(&mut wire)
    .unwrap();
    let request = confirmed(
        ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        46,
        wire.freeze(),
    );
    let admitted = f.capture(&mut a, &request).await;
    let identity = admitted.provenance.direct_sc_identity().unwrap();
    let npdu = decode_npdu(admitted.npdu.clone()).unwrap();

    let (transport, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut ingress = EndpointIngress::new(transport, 4);
    let receivers = ingress.start().await.unwrap();
    let observed = Arc::new(StdMutex::new(Vec::<ReinitializeContext>::new()));
    let recorded = observed.clone();
    let responder = EndpointResponder::new(
        Arc::new(RwLock::new(ObjectDatabase::new())),
        receivers.egress,
    )
    .with_reinitialize(
        Arc::new(
            move |context: &ReinitializeContext, _: &mut ObjectDatabase| {
                recorded.lock().unwrap().push(context.clone());
                Ok(())
            },
        ),
        None,
    );
    // The reply's direct capability belongs to the SC listener, not this
    // loopback egress, so the send may fail; the handler has run by then.
    let _ = responder
        .handle(ReceivedApdu {
            direct_response: None,
            apdu: npdu.payload,
            source_mac: admitted.source_mac.clone(),
            source_network: npdu.source,
            ingress_network: None,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: admitted.provenance,
            reply_tx: None,
        })
        .await;
    let context = observed.lock().unwrap().pop().unwrap();
    assert_eq!(context.provenance, admitted.provenance);
    assert_eq!(context.direct_sc_identity(), Some(identity));
    assert_eq!(context.invoke_id, 46);
    ingress.stop().await.unwrap();
    f.stop().await;
}
