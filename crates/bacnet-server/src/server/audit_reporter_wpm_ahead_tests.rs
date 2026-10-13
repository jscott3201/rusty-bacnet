//! Deciding a WritePropertyMultiple's durable attempts before staging them
//! (#1321) leaves the decision counters and the Audit records as the
//! handler alone made them: an attempt the request never reaches may be put
//! to the authorizer, but it is neither counted nor audited.

use super::*;
use crate::mutation::{MutationDecisionCounters, MutationServiceCounters};
use bacnet_encoding::constructed::encode_access_rule;
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::access_control::{
    AccessRightsObject, AccessRightsPersistence, AccessRightsSnapshot,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetAccessRule;
use std::sync::atomic::AtomicUsize;

/// What the Access Rights object last saved.
#[derive(Default)]
struct Saved(StdMutex<Option<AccessRightsSnapshot>>);

impl AccessRightsPersistence for Saved {
    fn load(&self, _rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        Ok(None)
    }

    fn save(
        &self,
        _rights: ObjectIdentifier,
        snapshot: &AccessRightsSnapshot,
    ) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn attempt(
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Vec<u8>,
) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: index,
        value,
        priority: None,
    }
}

/// Send a WritePropertyMultiple through the server's own dispatch, which
/// counts its decisions in the server's counters.
async fn dispatch_counted(server: &BACnetServer<TestTransport>, request: Bytes) -> Apdu {
    let (tx, rx) = oneshot::channel();
    BACnetServer::dispatch(
        &server.test_dispatch_context(),
        SOURCE,
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 9,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            service_request: request,
        }),
        ReceivedApdu {
            direct_response: None,
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: vec![],
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx: Some(tx),
        },
    )
    .await;
    let reply = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .expect("the request was answered")
        .unwrap();
    decode_apdu(decode_npdu(reply).unwrap().payload).unwrap()
}

#[tokio::test]
async fn deciding_attempts_ahead_counts_and_audits_only_the_attempts_reached() {
    let mut fixture = server(reporter()).await;
    let storage = Arc::new(Saved::default());
    let rights = AccessRightsObject::with_persistence(
        1,
        "AR",
        Arc::clone(&storage) as Arc<dyn AccessRightsPersistence>,
    )
    .unwrap();
    let target = rights.object_identifier();
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(rights))
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    fixture.server.config_mut().mutation_authorizer = Some(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        true
    }));
    let zone = oid(ObjectType::ACCESS_ZONE, 1);
    let mut rule = BytesMut::new();
    encode_access_rule(
        &mut rule,
        &BACnetAccessRule::new(None, Some(zone.into()), true),
    );
    // Enable FALSE, then an element of a property the object lacks, which
    // fails before the authorizer would see it, then a rules write the
    // request never reaches. The first and last were decided ahead.
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: target,
            list_of_properties: vec![
                attempt(PropertyIdentifier::LOG_ENABLE, None, vec![0x10]),
                attempt(PropertyIdentifier::from_raw(5555), Some(1), vec![0x10]),
                attempt(
                    PropertyIdentifier::POSITIVE_ACCESS_RULES,
                    None,
                    rule.to_vec(),
                ),
            ],
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    let Apdu::Error(error) = dispatch_counted(&fixture.server, bytes.freeze()).await else {
        panic!("expected a WritePropertyMultiple error");
    };
    assert_eq!(error.error_code, ErrorCode::UNKNOWN_PROPERTY);
    completed(&fixture).await;
    // Both attempts the objects save first were put to the authorizer ahead
    // of the handler, the one the request never reached included.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // Only the attempt the handler reached is counted and audited.
    assert_eq!(
        fixture.server.mutation_decision_counters(),
        MutationDecisionCounters {
            write_property_multiple: MutationServiceCounters {
                allow_total: 1,
                ..Default::default()
            },
            ..Default::default()
        }
    );
    let emitted = notifications(&fixture.transport.sent);
    let records: Vec<_> = emitted
        .iter()
        .flat_map(|request| &request.notifications)
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].target_object, Some(target));
    assert_eq!(
        records[0].target_property,
        Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::LOG_ENABLE,
            property_array_index: None
        })
    );
    // Storage holds what the object serves: Enable, and no rules. The
    // request released its staged rules itself, so settling, which would
    // put storage back too, finds nothing staged.
    let mut db = fixture.server.db.write().await;
    let writes = db
        .get_mut(&target)
        .and_then(|object| object.durable_writes_internal())
        .unwrap();
    assert!(!writes.has_staged_write(), "a write is still staged");
    let wait = writes.settle_forgotten_writes().unwrap();
    drop(db);
    tokio::time::timeout(Duration::from_secs(10), wait)
        .await
        .unwrap();
    assert_eq!(
        *storage.0.lock().unwrap(),
        Some(AccessRightsSnapshot {
            enable: Some(false),
            ..AccessRightsSnapshot::default()
        })
    );
}
