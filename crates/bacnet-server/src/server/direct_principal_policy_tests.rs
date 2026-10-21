//! `ServerPolicy` over real direct TLS (#524): the principal is the verified
//! leaf, so a policy tells two CA-valid peers apart, and a denied write
//! changes nothing. A routed request names only its forwarding peer.
use super::*;
use crate::mutation::{MutationAuthorizationContext, MutationServiceCounters};
use crate::server::channel_wire_tests::{channel, member};
use crate::server::command_action_wire_tests::ao;
use crate::ServerPolicy;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_transport::port::ScPrincipal;
use bacnet_types::enums::EngineeringUnits;

type Seen = Arc<StdMutex<Vec<(Option<ScPrincipal>, Option<ScPrincipal>)>>>;

/// Allows writes from one pinned principal; records what each call saw.
#[derive(Default)]
struct PinnedWriter {
    pinned: StdMutex<Option<ScPrincipal>>,
    seen: StdMutex<Vec<(Option<ScPrincipal>, String)>>,
}

impl ServerPolicy for PinnedWriter {
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        let principal = context.principal();
        self.seen
            .lock()
            .unwrap()
            .push((principal, format!("{context:?}")));
        principal.is_some() && principal == *self.pinned.lock().unwrap()
    }
}

fn write(text: &str, invoke: u8) -> Apdu {
    confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        invoke,
        write_payload(text),
    )
}

async fn present_value(f: &Fixture) -> PropertyValue {
    f.server
        .db
        .read()
        .await
        .get(&csv_oid())
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

/// Debug shows whether a principal is present, never any of its digits.
fn assert_redacted(debug: &str, principal: ScPrincipal) {
    assert!(debug.contains("principal: true"), "{debug}");
    assert!(!debug.contains(&principal.to_string()[..8]), "{debug}");
    assert!(!debug.contains("from "), "{debug}");
}

#[tokio::test]
async fn server_policy_tells_direct_leaf_principals_apart() {
    let ca = TestCa::new();
    let (a_tls, b_tls) = (ca.tls("a"), ca.tls("b"));
    let policy = Arc::new(PinnedWriter::default());
    let config = ServerConfig {
        policy: Some(policy.clone()),
        ..Default::default()
    };
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;

    // A's principal is the SHA-256 of its verified leaf, with no incarnation.
    let mut a = f.peer(a_tls.clone()).await;
    let from_a = f.capture_from(&mut a, &write("from A", 1), None).await;
    let a_identity = from_a.provenance.direct_sc_identity().unwrap();
    let a_principal = from_a.provenance.principal().unwrap();
    assert_eq!(a_principal.leaf_sha256(), a_identity.leaf_sha256());
    *policy.pinned.lock().unwrap() = Some(a_principal);
    f.feed(from_a).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    assert_eq!(
        present_value(&f).await,
        PropertyValue::CharacterString("from A".into())
    );

    // B, signed by the same CA and claiming the same VMAC and UUID, is a
    // different principal: denied, and nothing changes.
    let mut b = f.peer(b_tls).await;
    let from_b = f.capture_from(&mut b, &write("from B", 2), None).await;
    let b_principal = from_b.provenance.principal().unwrap();
    assert_ne!(b_principal, a_principal);
    f.feed(from_b).await;
    assert!(denied(&f.response().await));
    assert_eq!(
        present_value(&f).await,
        PropertyValue::CharacterString("from A".into())
    );

    // A reconnecting gets a new incarnation but keeps its principal.
    let mut again = f.peer(a_tls).await;
    let from_again = f.capture_from(&mut again, &write("A again", 3), None).await;
    let again_identity = from_again.provenance.direct_sc_identity().unwrap();
    assert_ne!(again_identity.incarnation(), a_identity.incarnation());
    assert_eq!(from_again.provenance.principal(), Some(a_principal));
    f.feed(from_again).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    assert_eq!(
        present_value(&f).await,
        PropertyValue::CharacterString("A again".into())
    );

    let seen = policy.seen.lock().unwrap().clone();
    let principals: Vec<_> = seen.iter().map(|(principal, _)| *principal).collect();
    assert_eq!(
        principals,
        [Some(a_principal), Some(b_principal), Some(a_principal)]
    );
    for (_, debug) in &seen {
        assert_redacted(debug, a_principal);
        assert_redacted(debug, b_principal);
    }
    assert_eq!(
        f.server.mutation_decision_counters().write_property,
        MutationServiceCounters {
            allow_total: 2,
            deny_total: 1,
            policy_deny_total: 0,
        }
    );
    f.stop().await;
}

#[tokio::test]
async fn a_routed_request_over_direct_tls_names_only_its_forwarding_peer() {
    let ca = TestCa::new();
    let seen = Seen::default();
    let record = seen.clone();
    let config = ServerConfig {
        policy: Some(Arc::new(crate::FnPolicy::new().mutation(move |context| {
            record
                .lock()
                .unwrap()
                .push((context.principal(), context.forwarding_peer_principal()));
            context.principal().is_some()
        }))),
        ..Default::default()
    };
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;
    let before = present_value(&f).await;
    let mut router = f.peer(ca.tls("router")).await;
    // `capture` sends the request with a routed SNET/SADR source.
    let routed = f.capture(&mut router, &write("routed", 1)).await;
    let router_principal = routed.provenance.principal().unwrap();
    f.feed(routed).await;
    assert!(denied(&f.response().await));
    assert_eq!(present_value(&f).await, before);
    assert_eq!(
        *seen.lock().unwrap(),
        [(None, Some(router_principal))],
        "a routed request's principal is unknown; only its forwarder's is"
    );
    f.stop().await;
}

fn channel_database(executions: Arc<AtomicUsize>) -> ObjectDatabase {
    let mut db = database(executions);
    db.add(Box::new(
        AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let mut object = channel(
        1,
        11,
        vec![(member(ao(1), PropertyIdentifier::PRESENT_VALUE), 0)],
    );
    object.set_control_groups(vec![27]).unwrap();
    db.add(Box::new(object)).unwrap();
    db
}

fn write_group() -> Apdu {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::Real(5.0)).unwrap();
    let mut service = BytesMut::new();
    WriteGroupRequest {
        group_number: std::num::NonZeroU32::new(27).unwrap(),
        write_priority: 8,
        change_list: vec![GroupChannelValue {
            channel: 11,
            override_priority: None,
            value: value.to_vec(),
        }],
        inhibit_delay: None,
    }
    .encode(&mut service)
    .unwrap();
    Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::WRITE_GROUP,
        service_request: service.freeze(),
    })
}

#[tokio::test]
async fn life_safety_and_write_group_contexts_carry_the_direct_principal() {
    let ca = TestCa::new();
    let seen = Seen::default();
    let (mutation, life_safety) = (seen.clone(), seen.clone());
    let config = ServerConfig {
        policy: Some(Arc::new(
            crate::FnPolicy::new()
                .mutation(move |context| {
                    mutation
                        .lock()
                        .unwrap()
                        .push((context.principal(), context.forwarding_peer_principal()));
                    true
                })
                .life_safety_operation(move |context| {
                    life_safety
                        .lock()
                        .unwrap()
                        .push((context.principal(), context.forwarding_peer_principal()));
                    true
                }),
        )),
        ..Default::default()
    };
    let executions = Arc::new(AtomicUsize::new(0));
    let mut f = Fixture::new(&ca, config, channel_database(executions.clone())).await;
    let mut a = f.peer(ca.tls("a")).await;

    let lso = f.capture_from(&mut a, &request(true, 1), None).await;
    let principal = lso.provenance.principal().unwrap();
    f.feed(lso).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    assert_eq!(executions.load(Ordering::Acquire), 1);

    let group = f.capture_from(&mut a, &write_group(), None).await;
    f.feed(group).await;
    // Nothing answers WriteGroup: wait for its one Channel decision.
    bounded(async {
        while seen.lock().unwrap().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (Some(principal), Some(principal)),
            (Some(principal), Some(principal))
        ]
    );
    f.stop().await;
}
