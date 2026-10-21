//! Hub-relayed requests carry no principal (#524): the hub authenticated its
//! own TLS clients, and the origin's certificate never reaches the server.
use super::support::{run, Certificates, PEERS};
use crate::mutation::MutationAuthorizationContext;
use crate::server::*;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ScPrincipal, TransportProvenance};
use std::sync::Mutex as StdMutex;

type Seen = Arc<StdMutex<Vec<(Option<ScPrincipal>, TransportProvenance)>>>;

fn description_write(text: &str) -> Bytes {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::CharacterString(text.into())).unwrap();
    let mut data = BytesMut::new();
    WritePropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut data)
    .unwrap();
    data.freeze()
}

#[tokio::test]
async fn sc_hub_relayed_mutation_has_no_principal() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        let seen = Seen::default();
        let record = seen.clone();
        // Writes only from a verified direct peer: nothing through a hub.
        let policy =
            crate::FnPolicy::new().mutation(move |context: &MutationAuthorizationContext| {
                record
                    .lock()
                    .unwrap()
                    .push((context.principal(), context.provenance));
                context.principal().is_some()
            });
        f.start(&certs, BACnetServer::sc_builder().policy(policy))
            .await;
        let description = async |f: &mut super::support::Fixture| {
            let db = f.server().database().read().await;
            db.get(&ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap())
                .unwrap()
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .ok()
        };
        let before = description(f).await;

        // Two hub members with different CA-valid leaves look the same.
        for peer in 0..PEERS.len() {
            let (id, response) = f
                .exchange(
                    peer,
                    ConfirmedServiceChoice::WRITE_PROPERTY,
                    description_write("relayed"),
                    None,
                )
                .await;
            let Apdu::Error(error) = response else {
                panic!("expected a denial, got {response:?}")
            };
            assert_eq!(error.invoke_id, id);
            assert_eq!(
                (error.error_class, error.error_code),
                (ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
            );
        }
        assert_eq!(description(f).await, before);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), PEERS.len());
        for (principal, provenance) in seen {
            assert_eq!(principal, None);
            assert!(!provenance.is_direct_peer());
            assert_eq!(provenance.direct_sc_identity(), None);
        }
        assert_eq!(
            f.server()
                .mutation_decision_counters()
                .write_property
                .deny_total,
            PEERS.len() as u64
        );
    })
    .await;
}
