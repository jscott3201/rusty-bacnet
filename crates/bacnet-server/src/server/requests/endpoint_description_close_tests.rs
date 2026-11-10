//! The endpoint's own Device keeps a written Description (#2364), and the
//! responder is closed between the save and the write: the request is
//! refused, the Device serves what it served, and storage goes back to it.

use super::*;
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use std::sync::Mutex;
use std::sync::mpsc;

/// Tags storage in memory that reports the Description of every save as
/// it lands, and can hold a save until the test lets it go.
struct Storage {
    landed: Mutex<mpsc::Sender<Option<String>>>,
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl TagsPersistence for Storage {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(None)
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        // A gate the test has dropped lets every later save through.
        if let Some((started, go)) = self.gate.lock().unwrap().take() {
            let _ = started.send(());
            let _ = go.recv();
        }
        let description = snapshot.description.clone();
        let _ = self.landed.lock().unwrap().send(description);
        Ok(())
    }
}

#[tokio::test]
async fn a_close_after_the_save_refuses_the_write_and_puts_storage_back() {
    let (started, started_rx) = mpsc::channel();
    let (go, go_rx) = mpsc::channel();
    let (landed, saves) = mpsc::channel();
    let storage = Arc::new(Storage {
        landed: Mutex::new(landed),
        gate: Mutex::new(Some((started, go_rx))),
    });
    // Storage answers at once; the bound only ends a failing test.
    let next_save = || saves.recv_timeout(std::time::Duration::from_secs(10));

    let (transport, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut ingress = EndpointIngress::new(transport, 4);
    let receivers = ingress.start().await.unwrap();
    let config = DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(123, 0).unwrap(),
        ..Default::default()
    };
    let mut object = DeviceObject::with_tags_persistence(config, storage.clone()).unwrap();
    object.set_description("configured");
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let responder = EndpointResponder::new(Arc::new(RwLock::new(db)), receivers.egress)
        .with_device_writes(device(), Arc::new(|_| true));

    let mut received = received(request(&write()));
    let (tx, rx) = oneshot::channel();
    received.reply_tx = Some(tx);
    let mut handle = Box::pin(responder.handle(received));
    // Poll to the wait for the staged save, which the storage holds.
    assert!(futures_util::poll!(&mut handle).is_pending());
    started_rx.recv().unwrap();
    responder.close();
    go.send(()).unwrap();
    handle.await.unwrap();

    // The close is answered as any refusal that isn't the object's own.
    let response = decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap();
    assert_error(response, ErrorClass::SERVICES, ErrorCode::OTHER);
    assert_eq!(
        description(&responder).await,
        PropertyValue::CharacterString("configured".into())
    );
    // Storage took the written text, and the release of the staged write
    // then put it back to what the Device serves: no Description saved.
    assert_eq!(next_save(), Ok(Some("updated".to_owned())));
    assert_eq!(next_save(), Ok(None));
    assert!(
        responder
            .handle(self::received(request(&write())))
            .await
            .is_err()
    );
    ingress.stop().await.unwrap();
}
