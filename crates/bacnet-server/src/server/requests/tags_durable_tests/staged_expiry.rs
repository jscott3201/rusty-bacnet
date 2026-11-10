//! The server's monotonic pass drops a staged Tags write whose request
//! vanished (#2179). The objects that keep Tags stage through their
//! `DurableWrites` capability, and most of them have no `Timed` one: the
//! pass reaches them through `DurableWrites::expire_staged`.

use super::*;
use std::time::Duration;

/// `bacnet_objects::durable::STAGED_WRITE_LIFETIME`, and the pass's period.
const STAGED_WRITE_LIFETIME: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_secs(1);

fn floor() -> TagsSnapshot {
    TagsSnapshot {
        tags: Some(vec![BACnetNameValue::valued(
            "floor",
            PropertyValue::Unsigned(3),
        )]),
        description: None,
    }
}

fn exhaust() -> TagsSnapshot {
    TagsSnapshot {
        tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
        description: None,
    }
}

async fn write_local(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    octets: &[u8],
) -> Result<(), Error> {
    server
        .write_local(
            &oid,
            TAGS,
            None,
            PropertyValue::ApplicationData(octets.to_vec()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

/// Whether storage comes to hold `expected` within [`WAIT`] of real time.
/// The wait runs on the blocking pool, so a paused clock stands still.
async fn comes_to_hold(storage: &Arc<Storage>, expected: TagsSnapshot) -> bool {
    let storage = Arc::clone(storage);
    tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + WAIT;
        while std::time::Instant::now() < deadline {
            if storage.load_saved().as_ref() == Some(&expected) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    })
    .await
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn every_profile_object_puts_back_a_staged_write_whose_request_vanished() {
    for kind in KINDS {
        let storage = Arc::new(Storage::default());
        let (server, _inbound) = server(kind, &storage).await;
        let server = Arc::new(server);
        let oid = ObjectIdentifier::new(kind.kind, 1).unwrap();
        write_local(&server, oid, FLOOR).await.unwrap();
        assert!(comes_to_hold(&storage, floor()).await, "{kind:?}");
        let (started, go) = storage.hold();
        let writing = tokio::spawn({
            let server = Arc::clone(&server);
            async move { write_local(&server, oid, EXHAUST).await }
        });
        tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
            .await
            .unwrap()
            .expect("the save started");
        // The request goes while its save is held. Its staged write stays
        // behind, and once the save lands storage holds Tags the object
        // never served.
        writing.abort();
        assert!(writing.await.unwrap_err().is_cancelled());
        drop(go);
        assert!(comes_to_hold(&storage, exhaust()).await, "{kind:?}");
        let start = tokio::time::Instant::now();
        tokio::time::sleep(STAGED_WRITE_LIFETIME - TICK).await;
        assert_eq!(storage.load_saved(), Some(exhaust()), "{kind:?}");
        // By a lifetime and a tick, the monotonic pass has dropped the
        // staged write and queued a save of the served Tags.
        tokio::time::sleep_until(start + STAGED_WRITE_LIFETIME + TICK + Duration::from_millis(500))
            .await;
        assert!(
            comes_to_hold(&storage, floor()).await,
            "{kind:?}: storage kept Tags the object never served"
        );
        let mut server = Arc::into_inner(server).expect("no other handle on the server");
        server.stop().await.unwrap();
    }
}
