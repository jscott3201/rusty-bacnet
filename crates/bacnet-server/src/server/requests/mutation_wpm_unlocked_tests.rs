//! A WritePropertyMultiple asks its policy with no database guard held
//! (#2032): a policy that blocks delays only its own request, and the
//! database changing between the decisions and the writes fails cleanly.

use super::mutation_tests::{Fixture, MutationAuthorizer, apdu, assert_denied, oid, value, wpm};
use super::*;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleError};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;

/// Another client's address, for the read that runs beside the write.
const OTHER: &[u8] = &[127, 0, 0, 2, 0xba, 0xc0];

fn description(object: ObjectIdentifier, text: &str) -> WriteAccessSpecification {
    WriteAccessSpecification {
        object_identifier: object,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            value: value(PropertyValue::CharacterString(text.into())),
            priority: None,
        }],
    }
}

/// A policy that allows everything, blocking its first call until the test
/// releases it. `entered` fires once that call is blocked.
struct Gate {
    authorizer: MutationAuthorizer,
    entered: tokio::sync::mpsc::UnboundedReceiver<()>,
    release: mpsc::Sender<()>,
    calls: Arc<AtomicUsize>,
}

fn gate() -> Gate {
    let (entered_tx, entered) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    Gate {
        authorizer: Arc::new(move |_| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                entered_tx.send(()).unwrap();
                // A blocking wait, as a Python callable on another thread
                // would make: the worker thread is stuck here.
                release_rx.lock().unwrap().recv().unwrap();
            }
            true
        }),
        entered,
        release,
        calls,
    }
}

/// Start `bytes` as a WritePropertyMultiple, and return once its policy is
/// blocked in its first call.
async fn blocked_write(
    fixture: &Arc<Fixture>,
    gate: &mut Gate,
    bytes: Bytes,
) -> tokio::task::JoinHandle<Option<Bytes>> {
    let writer = Arc::clone(fixture);
    let write = tokio::spawn(async move {
        writer
            .dispatch(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, bytes, 1)
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), gate.entered.recv())
        .await
        .expect("the policy was asked")
        .unwrap();
    write
}

fn wpm_error(response: Bytes) -> WritePropertyMultipleError {
    let Apdu::Error(error) = apdu(response) else {
        panic!("expected a WritePropertyMultiple error");
    };
    WritePropertyMultipleError::from_error_pdu(&error).unwrap()
}

// Multi-threaded: the blocked policy holds one worker thread, as a blocking
// callable does, and the read runs on the other.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blocked_wpm_policy_holds_no_lock_and_another_client_reads() {
    let mut gate = gate();
    let fixture = Arc::new(Fixture::new(Some(gate.authorizer.clone())));
    let first = oid(ObjectType::BINARY_VALUE, 1);
    let write = blocked_write(&fixture, &mut gate, wpm(vec![description(first, "new")])).await;

    // No database guard of any kind is held while the policy decides.
    drop(
        fixture
            .db
            .try_write()
            .expect("no guard held while the policy runs"),
    );
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: first,
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut request);
    let read = tokio::time::timeout(
        Duration::from_secs(10),
        fixture.dispatch_from(
            OTHER,
            ConfirmedServiceChoice::READ_PROPERTY,
            request.freeze(),
            2,
        ),
    )
    .await
    .expect("another client's read completes while the policy blocks")
    .unwrap();
    assert!(matches!(apdu(read), Apdu::ComplexAck(_)));
    assert!(!write.is_finished());

    gate.release.send(()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(10), write)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    assert_eq!(
        fixture.read(first, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("new".into())
    );
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_object_deleted_after_its_decision_fails_as_unknown_when_applied() {
    let mut gate = gate();
    let fixture = Arc::new(Fixture::new(Some(gate.authorizer.clone())));
    let first = oid(ObjectType::BINARY_VALUE, 1);
    let second = oid(ObjectType::BINARY_VALUE, 2);
    let bytes = wpm(vec![
        description(first, "prefix"),
        description(second, "gone"),
    ]);
    let write = blocked_write(&fixture, &mut gate, bytes).await;
    fixture.db.write().await.remove(&second).unwrap().unwrap();
    gate.release.send(()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(10), write)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Both were decided ahead; the handler finds the second gone.
    assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
    let error = wpm_error(response);
    assert_eq!(error.error_class, ErrorClass::OBJECT);
    assert_eq!(error.error_code, ErrorCode::UNKNOWN_OBJECT);
    assert_eq!(error.first_failed_write_attempt.object_identifier, second);
    // The prefix stays written, as before.
    assert_eq!(
        fixture.read(first, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("prefix".into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_object_created_after_the_decisions_is_denied_when_applied() {
    let mut gate = gate();
    let fixture = Arc::new(Fixture::new(Some(gate.authorizer.clone())));
    let first = oid(ObjectType::BINARY_VALUE, 1);
    let late = oid(ObjectType::BINARY_VALUE, 9);
    let bytes = wpm(vec![
        description(first, "prefix"),
        description(late, "undecided"),
    ]);
    let write = blocked_write(&fixture, &mut gate, bytes).await;
    fixture
        .db
        .write()
        .await
        .add(Box::new(
            bacnet_objects::binary::BinaryValueObject::new(9, "late").unwrap(),
        ))
        .unwrap();
    gate.release.send(()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(10), write)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Deciding ahead stopped at the missing object, so the policy never
    // saw it; the handler, finding it, denies rather than ask under the lock.
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
    assert_denied(
        response.clone(),
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        1,
    );
    assert_eq!(
        wpm_error(response)
            .first_failed_write_attempt
            .object_identifier,
        late
    );
    assert_eq!(
        fixture.read(first, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("prefix".into())
    );
    assert_ne!(
        fixture.read(late, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("undecided".into())
    );
}

/// `stop()` joins every request it admitted, and a request waiting on its
/// policy is one of them. The policy runs off the request task, so the
/// request's abort lands while the policy is still blocked and `stop()`
/// ends without waiting for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_while_a_wpm_policy_is_blocked() {
    let mut gate = gate();
    let authorizer = gate.authorizer.clone();
    let (transport, inbound) = crate::server::test_transport::TestTransport::inbound(4);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(100, 0).unwrap(),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        bacnet_objects::binary::BinaryValueObject::new(1, "one").unwrap(),
    ))
    .unwrap();
    let mut server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .transport(transport)
        .database(db)
        .enable_event_enrollment(false)
        .policy(crate::FnPolicy::new().mutation(move |context| authorizer(context)))
        .build()
        .await
        .unwrap();
    let first = oid(ObjectType::BINARY_VALUE, 1);
    super::durable_stop_tests::send(
        &inbound,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm(vec![description(first, "never")]),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), gate.entered.recv())
        .await
        .expect("the policy was asked")
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), server.stop())
        .await
        .expect("stop ends while the policy is blocked")
        .unwrap();
    // The abandoned call finishes on its own thread; nothing is written.
    gate.release.send(()).unwrap();
    assert_ne!(
        server
            .db
            .read()
            .await
            .get(&first)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("never".into())
    );
}
