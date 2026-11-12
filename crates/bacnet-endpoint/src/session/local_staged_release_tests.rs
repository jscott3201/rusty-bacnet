//! A local write refused before it reaches the object releases the durable
//! write it staged, under the guard that refuses it (#2468 review), as the
//! server's does (`local_index_staged_release_tests` in bacnet-server):
//! storage goes back to the state the object serves, and the next write to
//! the object finds it free.
//!
//! No bundled object stages a write that the local write's checks then
//! refuse, so NF-1 here is a Notification Forwarder whose staging ignores
//! the index: a Recipient_List[1] write stages a real save of the list
//! before the index check refuses it. It counts its releases, so the test
//! sees the release made by the refusing call itself, not one made later
//! from a task (a dropped call's path).

use super::*;
use crate::session::bus_port::BusPort;
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_objects::durable::{DurableWrites, SaveWait, StageStep};
use bacnet_objects::notification_forwarder::{
    ForwarderSnapshot, NotificationForwarderObject, NotificationForwarderPersistence,
};
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::Time;
use std::borrow::Cow;
use std::net::Ipv4Addr;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(3);

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

/// A forwarder that stages a write whatever its index, and counts the
/// releases it is handed.
struct IndexBlind {
    forwarder: NotificationForwarderObject,
    releases: Arc<AtomicUsize>,
}

impl BACnetObject for IndexBlind {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.forwarder.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.forwarder.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.forwarder.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.forwarder
            .write_property(property, array_index, value, priority)
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.forwarder.property_metadata()
    }

    fn durable_writes(&mut self) -> Option<&mut dyn DurableWrites> {
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
        self.forwarder
            .durable_writes()
            .expect("a forwarder saves its lists")
            .stage_write(property, None, value)
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.releases.fetch_add(1, Ordering::SeqCst);
        if let Some(writes) = self.forwarder.durable_writes() {
            writes.release_staged_write(staged);
        }
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        self.forwarder.durable_writes()?.settle_forgotten_writes()
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
    let mut encoded = bytes::BytesMut::new();
    encode_destination_list(&mut encoded, destinations).unwrap();
    PropertyValue::ApplicationData(encoded.to_vec())
}

async fn write(
    session: &EndpointSession<BusPort>,
    index: Option<u32>,
    destinations: &[BACnetDestination],
) -> Result<(), Error> {
    let source = LocalCommandSource::ServerDevice;
    let property = PropertyIdentifier::RECIPIENT_LIST;
    let value = list(destinations);
    let forwarder = nf();
    let write = session.write_local(&forwarder, property, index, value, None, source);
    tokio::time::timeout(WAIT, write)
        .await
        .expect("the write ends")
}

/// The Recipient_List NF-1's storage holds once its saves have run.
async fn stored(saved: &Saved, expected: Option<Vec<BACnetDestination>>) {
    let held = || {
        let snapshot = saved.0.lock().unwrap();
        snapshot
            .as_ref()
            .map(|snapshot| snapshot.recipient_list.clone())
    };
    tokio::time::timeout(WAIT, async {
        while held() != Some(expected.clone()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("storage holds {:?}, not {expected:?}", held()));
}

/// Real time on one thread: a release made later from a task can't have
/// run by the time the refused call returns.
#[tokio::test]
async fn a_local_write_refused_before_the_object_releases_what_it_staged() {
    let saved = Arc::new(Saved::default());
    let persistence = Arc::clone(&saved) as Arc<dyn NotificationForwarderPersistence>;
    let forwarder = NotificationForwarderObject::with_persistence(1, "NF-1", persistence).unwrap();
    let releases = Arc::new(AtomicUsize::new(0));
    let identity = crate::DeviceIdentity::new(1426, 0)
        .unwrap()
        .with_name("Staged release");
    let blind = IndexBlind {
        forwarder,
        releases: Arc::clone(&releases),
    };
    let db = crate::identity::build_database_with_extra(&identity, vec![Box::new(blind)]).unwrap();
    let mut session =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(db)
            .build_session_over(BusPort::new())
            .unwrap();
    session.start().await.unwrap();

    // The list is staged and saved, then the index check refuses the write
    // and the call releases the stage before it returns.
    let error = write(&session, Some(1), &[destination(7)])
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        "{error:?}"
    );
    assert_eq!(releases.load(Ordering::SeqCst), 1, "released by the call");
    // Released, storage goes back to the list NF-1 serves: none written.
    stored(&saved, None).await;

    // The next write finds NF-1 free and saves its own list.
    write(&session, None, &[destination(8)]).await.unwrap();
    stored(&saved, Some(vec![destination(8)])).await;
    session.stop().await.unwrap();
}
