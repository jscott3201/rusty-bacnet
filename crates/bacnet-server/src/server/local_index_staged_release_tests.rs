//! A local write refused before it reaches the object releases the durable
//! write it staged (#1426 review): storage goes back to the state the object
//! serves, and the next write to the object doesn't find it busy.
//!
//! No bundled object stages a write that `precheck` then refuses: each
//! skips an indexed write, and none saves Object_Name. So NF-1 here is a
//! Notification Forwarder whose staging ignores the index. A
//! Recipient_List[1] write therefore stages a real save of the list before
//! the index gate refuses it.

use super::super::*;
use crate::server::clock::clocked_test_database;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::durable::{DurableWrites, SaveWait, StageStep};
use bacnet_objects::notification_forwarder::{
    ForwarderSnapshot, NotificationForwarderObject, NotificationForwarderPersistence,
};
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::Time;
use std::borrow::Cow;
use std::sync::Mutex as StdMutex;

const WAIT: Duration = Duration::from_secs(10);

/// What NF-1 last saved.
#[derive(Default)]
struct Saved(StdMutex<Option<ForwarderSnapshot>>);

impl NotificationForwarderPersistence for Saved {
    fn load(&self, _forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error> {
        Ok(None)
    }

    fn save(
        &self,
        _forwarder: ObjectIdentifier,
        snapshot: &ForwarderSnapshot,
    ) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

/// A forwarder that stages a write whatever its index.
struct IndexBlind(NotificationForwarderObject);

impl BACnetObject for IndexBlind {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.0.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.0.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0
            .write_property(property, array_index, value, priority)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.0.property_list()
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.0.property_metadata()
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn DurableWrites> {
        Some(self)
    }
}

impl DurableWrites for IndexBlind {
    fn stage_write(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep {
        self.0
            .durable_writes_internal()
            .expect("a forwarder saves its lists")
            .stage_write(property, None, value)
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        if let Some(writes) = self.0.durable_writes_internal() {
            writes.release_staged_write(staged);
        }
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        self.0.durable_writes_internal()?.settle_forgotten_writes()
    }
}

fn nf() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap()
}

fn destination(device: u32) -> BACnetDestination {
    let time = |hour, minute, second, hundredths| Time {
        hour,
        minute,
        second,
        hundredths,
    };
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: time(0, 0, 0, 0),
        to_time: time(23, 59, 59, 99),
        recipient: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap(),
        ),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

fn list(destinations: &[BACnetDestination]) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode_destination_list(&mut encoded, destinations).unwrap();
    PropertyValue::ApplicationData(encoded.to_vec())
}

async fn write(
    server: &BACnetServer<TestTransport>,
    index: Option<u32>,
    destinations: &[BACnetDestination],
) -> Result<(), Error> {
    tokio::time::timeout(
        WAIT,
        server.write_local(
            &nf(),
            PropertyIdentifier::RECIPIENT_LIST,
            index,
            list(destinations),
            None,
            crate::LocalCommandSource::ServerDevice,
        ),
    )
    .await
    .expect("the write ends")
}

/// The Recipient_List NF-1's storage holds once its saves have run.
async fn stored(saved: &Saved, expected: Option<Vec<BACnetDestination>>) {
    tokio::time::timeout(WAIT, async {
        while saved
            .0
            .lock()
            .unwrap()
            .as_ref()
            .map(|snapshot| snapshot.recipient_list.clone())
            != Some(expected.clone())
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("storage never held {expected:?}"));
}

#[tokio::test]
async fn a_local_write_refused_before_the_object_releases_what_it_staged() {
    let saved = Arc::new(Saved::default());
    let forwarder = NotificationForwarderObject::with_persistence(
        1,
        "NF-1",
        Arc::clone(&saved) as Arc<dyn NotificationForwarderPersistence>,
    )
    .unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(1426, 0)
                .expect("valid Device identity")
                .with_name("Staged release"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(IndexBlind(forwarder))).unwrap();
    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(1426, 0).unwrap())
        .transport(TestTransport::new())
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();

    // The list is staged and saved, then the index gate refuses the write.
    let error = write(&server, Some(1), &[destination(7)])
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        "{error:?}"
    );
    // Released, it goes back to the list NF-1 serves: none written yet.
    stored(&saved, None).await;

    // The next write finds NF-1 free and saves its own list.
    write(&server, None, &[destination(8)]).await.unwrap();
    assert_eq!(
        super::super::durable_writes::busy_waits(server.database()),
        0
    );
    stored(&saved, Some(vec![destination(8)])).await;
}
