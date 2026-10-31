//! Timer countdowns on the server's monotonic operation task (#2192), in
//! paused time: a start written over the wire wakes the task, which expires
//! the timer at its deadline, holds it while out of service, and carries on
//! when a running timer is deleted.

use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::timer::TimerObject;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::{TimerState, TimerTransition};

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

struct Wire {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    invoke_id: u8,
}

fn timer_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TIMER, instance).unwrap()
}

impl Wire {
    /// A server with Timers 1 and 2, each starting with 400 ms when
    /// Timer_Running is written TRUE, and run 1.2 s: past the task's first
    /// pass and its 1 s one. That tick fires late, at 1.2 s, and the task's
    /// interval delays the next routine pass to 2.2 s.
    async fn start() -> Self {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(2192, 0)
                    .expect("valid Device identity")
                    .with_name("Timer-task-device"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        for instance in [1, 2] {
            let mut timer = TimerObject::new(instance, format!("TMR-{instance}")).unwrap();
            timer.set_pres_value_range(100, 600_000).unwrap();
            timer.set_default_timeout(400).unwrap();
            db.add(Box::new(timer)).unwrap();
        }
        let config = ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(2192, 0).unwrap(),
            enable_event_enrollment: false,
            ..ServerConfig::default()
        };
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder()
            .local_mac(&[0x0A, 0, 0, 2, 0xBA, 0xC0])
            .inbound(rx)
            .build();
        let server = BACnetServer::start(config, db, transport).await.unwrap();
        settle().await;
        advance(1_200).await;
        Self {
            server,
            tx,
            invoke_id: 0,
        }
    }

    /// One confirmed request from a peer; the reply PDU.
    async fn send(&mut self, service_choice: ConfirmedServiceChoice, request: BytesMut) -> Apdu {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let mut payload = BytesMut::new();
        encode_apdu(
            &mut payload,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: self.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_request: request.freeze(),
            }),
        )
        .unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                expecting_reply: true,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&[0x0A, 0, 0, 5, 0xBA, 0xC0]),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: Some(reply_tx),
            })
            .await
            .unwrap();
        let reply = reply_rx.await.unwrap();
        apdu::decode_apdu(decode_npdu(reply).unwrap().payload).unwrap()
    }

    /// A WriteProperty over the wire, which must be acknowledged.
    async fn write(&mut self, instance: u32, property: PropertyIdentifier, value: PropertyValue) {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut request = BytesMut::new();
        WritePropertyRequest {
            object_identifier: timer_oid(instance),
            property_identifier: property,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: None,
        }
        .encode(&mut request)
        .unwrap();
        let reply = self
            .send(ConfirmedServiceChoice::WRITE_PROPERTY, request)
            .await;
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    }

    async fn delete(&mut self, instance: u32) {
        let mut request = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: timer_oid(instance),
        }
        .encode(&mut request);
        let reply = self
            .send(ConfirmedServiceChoice::DELETE_OBJECT, request)
            .await;
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    }

    /// Timer_State, Last_State_Change and the armed deadline, as the
    /// object holds them: a deadline only the task's pass clears.
    async fn state(&self, instance: u32) -> (PropertyValue, PropertyValue, Option<Duration>) {
        let db = self.server.database().read().await;
        let timer = db.get(&timer_oid(instance)).unwrap();
        (
            timer
                .read_property(PropertyIdentifier::TIMER_STATE, None)
                .unwrap(),
            timer
                .read_property(PropertyIdentifier::LAST_STATE_CHANGE, None)
                .unwrap(),
            timer.next_monotonic_deadline_internal(),
        )
    }

    async fn present_value(&self, instance: u32) -> PropertyValue {
        let db = self.server.database().read().await;
        db.get(&timer_oid(instance))
            .unwrap()
            .read_property(PV, None)
            .unwrap()
    }
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Move paused time on by `milliseconds` and let the server catch up.
async fn advance(milliseconds: u64) {
    tokio::time::advance(Duration::from_millis(milliseconds)).await;
    settle().await;
}

fn state(value: TimerState) -> PropertyValue {
    PropertyValue::Enumerated(value.to_raw())
}

fn transition(value: TimerTransition) -> PropertyValue {
    PropertyValue::Enumerated(value.to_raw())
}

fn expired() -> (PropertyValue, PropertyValue, Option<Duration>) {
    (
        state(TimerState::EXPIRED),
        transition(TimerTransition::RUNNING_TO_EXPIRED),
        None,
    )
}

#[tokio::test(start_paused = true)]
async fn a_start_over_the_wire_expires_at_its_deadline_not_the_next_tick() {
    let mut wire = Wire::start().await;
    // 300 ms, written at 1.2 s: due at 1.5 s, before the 2.2 s routine pass,
    // so only the start's wake gets the task there in time.
    wire.write(1, PV, PropertyValue::Unsigned(300)).await;
    let (running, started, deadline) = wire.state(1).await;
    assert_eq!(running, state(TimerState::RUNNING));
    assert_eq!(started, transition(TimerTransition::IDLE_TO_RUNNING));
    let deadline = deadline.expect("armed");
    advance(299).await;
    assert_eq!(wire.state(1).await.2, Some(deadline), "not early");
    assert_eq!(wire.present_value(1).await, PropertyValue::Unsigned(1));
    advance(1).await;
    assert_eq!(wire.state(1).await, expired());
    assert_eq!(wire.present_value(1).await, PropertyValue::Unsigned(0));

    // Timer_Running TRUE starts with the 400 ms Default_Timeout, at 1.5 s.
    wire.write(
        1,
        PropertyIdentifier::TIMER_RUNNING,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(
        wire.state(1).await.1,
        transition(TimerTransition::EXPIRED_TO_RUNNING)
    );
    advance(399).await;
    assert_eq!(wire.state(1).await.0, state(TimerState::RUNNING));
    advance(1).await;
    assert_eq!(wire.state(1).await, expired());
    wire.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn out_of_service_holds_the_countdown_and_resume_shifts_the_deadline() {
    let mut wire = Wire::start().await;
    wire.write(1, PV, PropertyValue::Unsigned(2_000)).await;
    advance(500).await;
    let oos = PropertyIdentifier::OUT_OF_SERVICE;
    wire.write(1, oos, PropertyValue::Boolean(true)).await;
    // Held: no deadline, and no expiry however long it waits.
    assert_eq!(wire.state(1).await.2, None);
    advance(5_000).await;
    assert_eq!(wire.state(1).await.0, state(TimerState::RUNNING));
    assert_eq!(wire.present_value(1).await, PropertyValue::Unsigned(1_500));
    // Back in service the 1.5 s left runs from now.
    wire.write(1, oos, PropertyValue::Boolean(false)).await;
    advance(1_499).await;
    assert_eq!(wire.state(1).await.0, state(TimerState::RUNNING));
    assert_eq!(wire.present_value(1).await, PropertyValue::Unsigned(1));
    advance(1).await;
    assert_eq!(wire.state(1).await, expired());
    wire.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn deleting_a_running_timer_leaves_the_others_on_time() {
    let mut wire = Wire::start().await;
    wire.write(1, PV, PropertyValue::Unsigned(300)).await;
    wire.write(2, PV, PropertyValue::Unsigned(600)).await;
    advance(100).await;
    wire.delete(1).await;
    assert!(
        wire.server
            .database()
            .read()
            .await
            .get(&timer_oid(1))
            .is_none()
    );
    // Timer 1's deadline passes with nothing to expire.
    advance(200).await;
    assert_eq!(wire.state(2).await.0, state(TimerState::RUNNING));
    advance(299).await;
    assert_eq!(wire.state(2).await.0, state(TimerState::RUNNING));
    advance(1).await;
    assert_eq!(wire.state(2).await, expired());
    // The task still answers: a new start is served on time.
    wire.write(2, PV, PropertyValue::Unsigned(250)).await;
    advance(250).await;
    assert_eq!(wire.state(2).await, expired());
    wire.server.stop().await.unwrap();
}
