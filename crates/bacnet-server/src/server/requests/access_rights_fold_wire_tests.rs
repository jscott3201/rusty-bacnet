//! An Access Rights object's staged saves through the server: a request
//! that stops part way puts storage back to what the object serves (#1423),
//! and a write the object won't make leaves another request's staged save
//! in place (#1424).

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_refused_at_its_second_write_puts_storage_back() {
    let storage = Arc::new(RightsStorage::default());
    let (fixture, rights) = served_by(&storage).await;
    let first = [zone_rule(1), zone_rule(2)];
    // The first and third attempts stage one save together. The second is
    // refused, since Description takes a character string, so the request
    // makes its first write and never its third.
    let request = write_property_multiple(
        rights,
        vec![
            attempt(P::POSITIVE_ACCESS_RULES, None, encoded(&first)),
            attempt(P::DESCRIPTION, None, value(PropertyValue::Unsigned(1))),
            attempt(P::LOG_ENABLE, None, value(PropertyValue::Boolean(false))),
        ],
    );
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    assert_eq!(wire(&fixture, service, request).await[0], ERROR_PDU);
    saves_done(&fixture, rights).await;
    // Storage holds what the object serves: the first write, not the third.
    let served = positive_only(&first);
    assert_eq!(storage.load_saved(), Some(served.clone()));
    assert_eq!(reads(&fixture, rights).await, expected_reads(&served));
    // The staged save, then the one putting storage back.
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_another_path_cannot_make_leaves_a_staged_save_to_its_request() {
    let storage = Arc::new(RightsStorage::default());
    let (fixture, rights) = served_by(&storage).await;
    let rules = [zone_rule(3)];
    let (started, go) = storage.hold();
    let request = write_property(rights, P::POSITIVE_ACCESS_RULES, None, encoded(&rules));
    let sending = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        async move { wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    // While the save runs off the guard, application code writes through the
    // database: an element past the end, which the object refuses, and a
    // NULL Enable, which it refuses as the wrong datatype and the server
    // would answer as a success that changes nothing (#1396).
    let written = tokio::time::timeout(WAIT, fixture.db.write()).await;
    {
        let mut db = written.expect("the database was held while it saved");
        let object = db.get_mut(&rights).unwrap();
        let past_end = PropertyValue::ApplicationData(encoded(&[zone_rule(9)]));
        assert!(
            object
                .write_property(P::POSITIVE_ACCESS_RULES, Some(5), past_end, None)
                .is_err()
        );
        assert!(
            object
                .write_property(P::LOG_ENABLE, None, PropertyValue::Null, None)
                .is_err()
        );
    }
    // Let every save through, so one queued meanwhile can't hold the test.
    drop(go);
    assert_eq!(sending.await.unwrap(), SIMPLE_ACK_WRITE);
    saves_done(&fixture, rights).await;
    // The request took the save it staged: nothing saved in place, and
    // nothing put storage back in between.
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert_eq!(storage.load_saved(), Some(positive_only(&rules)));
    assert_eq!(
        reads(&fixture, rights).await,
        expected_reads(&positive_only(&rules))
    );
}

/// Under an authorizer, attempts the objects save first are decided before
/// they are staged, and the decisions end where the request will stop
/// (#1321): at an attempt whose value doesn't decode, as well as at one
/// denied.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undecodable_rights_attempt_ends_what_is_decided_and_staged() {
    let storage = Arc::new(RightsStorage::default());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let fixture = Fixture::new(Some(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        true
    })));
    let object = rights_object(&storage);
    let rights = object.object_identifier();
    fixture.db.write().await.add(Box::new(object)).unwrap();
    let fixture = Arc::new(fixture);
    // An array's size, at index 0, takes an Unsigned, and the first attempt
    // carries no octets at all. The request stops there, so Enable after it
    // is never put to the authorizer, staged or saved.
    let request = write_property_multiple(
        rights,
        vec![
            attempt(P::POSITIVE_ACCESS_RULES, Some(0), Vec::new()),
            attempt(P::LOG_ENABLE, None, value(PropertyValue::Boolean(true))),
        ],
    );
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    let response = wire(&fixture, service, request).await;
    let Apdu::Error(error) = bacnet_encoding::apdu::decode_apdu(Bytes::from(response)).unwrap()
    else {
        panic!("expected a WritePropertyMultiple error");
    };
    let refused = bacnet_services::wpm::WritePropertyMultipleError::from_error_pdu(&error).unwrap();
    assert_eq!(
        (refused.error_class, refused.error_code),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
    );
    let failed = refused.first_failed_write_attempt;
    assert_eq!(failed.object_identifier, rights);
    assert_eq!(
        failed.property_identifier,
        P::POSITIVE_ACCESS_RULES.to_raw()
    );
    assert_eq!(failed.property_array_index, Some(0));
    saves_done(&fixture, rights).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    assert_eq!(storage.load_saved(), None);
}
