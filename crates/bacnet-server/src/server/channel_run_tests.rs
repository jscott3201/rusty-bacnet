//! A Channel's distribution, on a running server: what happens when the
//! Channel goes away mid-run, what a member receives, and the order of
//! members whose delays are equal (#1151 review).
//!
//! These use the fixtures of `channel_wire_tests`: CH-3 writes AV-1 at once,
//! AO-9 (missing) after 100 ms and AO-2 after 200 ms, at the written
//! priority. The clock is paused.
use super::channel_wire_tests::{
    ch, channel, member, settled, slot, start, write_channel, write_status,
};
use super::command_action_wire_tests::ao;
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::WriteStatus;
use std::borrow::Cow;
use std::sync::Mutex as StdMutex;

type Log = Arc<StdMutex<Vec<(PropertyIdentifier, PropertyValue)>>>;

/// A vendor-type object that holds a REAL in every property and takes any
/// write, logging each one in order.
struct Recorder {
    oid: ObjectIdentifier,
    log: Log,
}

fn rec() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::from_raw(600), 1).unwrap()
}

impl BACnetObject for Recorder {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "REC-1"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(self.oid),
            PropertyIdentifier::OBJECT_NAME => PropertyValue::CharacterString("REC-1".into()),
            PropertyIdentifier::OBJECT_TYPE => {
                PropertyValue::Enumerated(self.oid.object_type().to_raw())
            }
            _ => PropertyValue::Real(0.0),
        })
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        self.log.lock().unwrap().push((property, value));
        Ok(())
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        // Vendor members 1000 to 1003 and Lighting_Command, all writable.
        let mut served: Vec<_> = (1000..=1003).map(PropertyIdentifier::from_raw).collect();
        served.push(PropertyIdentifier::LIGHTING_COMMAND);
        crate::test_rows::rows(&served, &served)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        bacnet_objects::property_metadata::property_list_from_metadata(&self.property_metadata())
    }
}

/// A server with REC-1 and CH-8, whose members `members` all name REC-1.
async fn start_recording(members: Vec<(PropertyIdentifier, u32)>) -> (Harness, Log) {
    let log = Log::default();
    let recorder = Recorder {
        oid: rec(),
        log: Arc::clone(&log),
    };
    let h = Harness::start_with(ServerConfig::default(), move |db| {
        db.add(Box::new(recorder)).unwrap();
        db.add(Box::new(channel(
            8,
            1,
            members
                .into_iter()
                .map(|(property, delay)| (member(rec(), property), delay))
                .collect(),
        )))
        .unwrap();
    })
    .await;
    (h, log)
}

#[tokio::test(start_paused = true)]
async fn channel_deleted_during_a_delay_leaves_the_rest_of_its_members_unwritten() {
    // REC-1 takes writes from any initiator, so only the run's own check
    // keeps the second member unwritten once CH-8 is gone.
    let property = PropertyIdentifier::from_raw;
    let (mut h, log) = start_recording(vec![(property(1000), 0), (property(1001), 100)]).await;
    write_channel(&mut h, 8, &PropertyValue::Real(30.0), Some(10))
        .await
        .unwrap();
    h.settle().await;
    assert_eq!(log.lock().unwrap().len(), 1);
    h.server
        .database()
        .write()
        .await
        .remove(&ch(8))
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].0, property(1000));
}

#[tokio::test(start_paused = true)]
async fn channel_replaced_during_a_delay_leaves_the_new_object_to_its_own_run() {
    let mut h = start().await;
    write_channel(&mut h, 3, &PropertyValue::Real(30.0), Some(10))
        .await
        .unwrap();
    h.settle().await;
    // The application puts a new CH-3 in place, writing AO-1 after a second,
    // and a client writes it once while the old run waits.
    h.server
        .database()
        .write()
        .await
        .add(Box::new(channel(3, 12, vec![(member(ao(1), PV), 1000)])))
        .unwrap();
    write_channel(&mut h, 3, &PropertyValue::Real(5.0), Some(10))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The old run neither wrote AO-2 nor ended the new object's run.
    assert_eq!(slot(&h, ao(2), 10).await, PropertyValue::Null);
    assert_eq!(write_status(&mut h, 3).await, WriteStatus::IN_PROGRESS);
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert_eq!(settled(&mut h, 3).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot(&h, ao(1), 10).await, PropertyValue::Real(5.0));
}

#[tokio::test(start_paused = true)]
async fn channel_lighting_command_reaches_its_member_without_the_context_0_framing() {
    let (mut h, log) = start_recording(vec![(PropertyIdentifier::LIGHTING_COMMAND, 0)]).await;
    // Operation 1 with a target level of 50.0, framed in [0].
    let framed = [0x0E, 0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F];
    write_channel(
        &mut h,
        8,
        &PropertyValue::ApplicationData(framed.to_vec()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(settled(&mut h, 8).await, WriteStatus::SUCCESSFUL);
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1);
    let (property, value) = &log[0];
    assert_eq!(*property, PropertyIdentifier::LIGHTING_COMMAND);
    // The member gets what a WriteProperty carrying the SEQUENCE alone
    // decodes to: the SEQUENCE's octets in one piece.
    assert_eq!(
        *value,
        PropertyValue::ApplicationData(framed[1..8].to_vec())
    );
}

#[tokio::test(start_paused = true)]
async fn channel_members_with_equal_delays_go_in_list_order() {
    let property = PropertyIdentifier::from_raw;
    let (mut h, log) = start_recording(vec![
        (property(1000), 50),
        (property(1001), 0),
        (property(1002), 50),
        (property(1003), 0),
    ])
    .await;
    write_channel(&mut h, 8, &PropertyValue::Real(2.5), Some(9))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(settled(&mut h, 8).await, WriteStatus::SUCCESSFUL);
    let order: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .map(|(property, value)| {
            assert_eq!(*value, PropertyValue::Real(2.5));
            property.to_raw()
        })
        .collect();
    assert_eq!(order, [1001, 1003, 1000, 1002]);
}
