//! A write and the event-state transition it causes are one timestamped
//! row (#2535): the server and the `Reporter` send the same reports, octet
//! for octet, for a request's fault and its clear, a value over the high
//! limit by WriteProperty and by a WritePropertyMultiple of one, two writes
//! in one WritePropertyMultiple both ways, one over the limit followed by a
//! write no reference conveys, and a local write.
//!
//! The fixture's AI-3 reports intrinsically (limits 0 and 80, deadband 1,
//! Time_Delay 0) with an empty Event_Enable, so a transition commits and
//! changes Status_Flags while no event notification goes out. One context
//! subscribes its Status_Flags and Present_Value, both timestamped, with a
//! Max_Notification_Delay of 0. Both sides read one fixed clock, so values
//! repeated at one time go out once, after the last row carrying them.
use super::super::local_write::cases::Change;
use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov_multiple::{
    COVNotificationMultipleRequest, COVReference, COVSubscriptionSpecification,
    SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::PropertyReference;

const OOS: u8 = 0x10;
const FAULT: u8 = 0x40;
const IN_ALARM: u8 = 0x80;

fn alarm() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 3)
}

/// The parity database with AI-3, reading 10.
fn alarm_fixture(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let mut object = AnalogInputObject::new(3, "AI-3", EngineeringUnits::NO_UNITS).unwrap();
    for (property, value) in [
        (P::HIGH_LIMIT, 80.0f32),
        (P::LOW_LIMIT, 0.0),
        (P::DEADBAND, 1.0),
    ] {
        let value = PropertyValue::Real(value);
        object.write_property(property, None, value, None).unwrap();
    }
    let both = PropertyValue::BitString {
        unused_bits: 6,
        data: vec![0xC0],
    };
    object
        .write_property(P::LIMIT_ENABLE, None, both, None)
        .unwrap();
    object.set_present_value(10.0);
    database_with(identity, vec![Box::new(object)])
}

/// A value a report carries: the property and its octets.
type Value = (P, Vec<u8>);

fn flags(bits: u8) -> Value {
    (P::STATUS_FLAGS, vec![0x82, 0x04, bits])
}

fn real(value: f32) -> Value {
    let mut octets = vec![0x44];
    octets.extend_from_slice(&value.to_be_bytes());
    (P::PRESENT_VALUE, octets)
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value).unwrap();
    encoded.to_vec()
}

/// The values of the COV-multiple reports among `frames`, in order.
fn values(frames: &[test_transport::SentFrame]) -> Vec<Value> {
    let mut values = Vec::new();
    for frame in frames {
        let apdu = bacnet_encoding::apdu::decode_apdu(frame.decode_npdu().payload).unwrap();
        let Apdu::UnconfirmedRequest(request) = apdu else {
            panic!("an unconfirmed report: {apdu:?}");
        };
        let report = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
        for item in report.list_of_cov_notifications {
            assert_eq!(item.monitored_object_identifier, alarm());
            for value in item.list_of_values {
                assert!(value.time_of_change.is_some(), "{value:?}");
                values.push((value.property_identifier, value.value));
            }
        }
    }
    values
}

impl CovParity {
    /// Both sides of [`alarm_fixture`] with the `Reporter`'s event path on
    /// and AI-3's context subscribed, its initial report sent.
    async fn start_alarm() -> Self {
        let mut cov = Self::start_from(CovPolicy::default(), alarm_fixture).await;
        // Start installs the system clock; serve the fixed one.
        cov.parity
            .server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        cov.event_path_on();
        let reference = |property| COVReference {
            monitored_property: PropertyReference {
                property_identifier: property,
                property_array_index: None,
            },
            cov_increment: None,
            timestamped: true,
        };
        let mut body = BytesMut::new();
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 1,
            issue_confirmed_notifications: false,
            lifetime: Some(300),
            max_notification_delay: Some(0),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: alarm(),
                list_of_cov_references: vec![
                    reference(P::STATUS_FLAGS),
                    reference(P::PRESENT_VALUE),
                ],
            }],
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
        let (reply, initial) = cov.exchange("subscribe", &direct(1), service, &body).await;
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
        assert_eq!(initial, 1, "the initial report");
        cov
    }

    /// Send `service` with `body` to both: each acknowledges it, their
    /// reports match, and the values the server's carry.
    async fn written(
        &mut self,
        what: &str,
        service: ConfirmedServiceChoice,
        body: &[u8],
    ) -> Vec<Value> {
        let now = runtime_clock::core_now();
        let (frames, core) = self.both(&direct(1), service, body).await;
        let (reply, reports) = frames
            .split_first()
            .unwrap_or_else(|| panic!("{what}: the server sent nothing"));
        assert_eq!(
            reply.decode_npdu().payload,
            core,
            "{what}: server, then core"
        );
        let reply = bacnet_encoding::apdu::decode_apdu(core).unwrap();
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{what}: {reply:?}");
        self.same_notifications(what, reports, now);
        values(reports)
    }

    /// WriteProperty of AI-3's `property`.
    async fn write_alarm(&mut self, what: &str, property: P, value: PropertyValue) -> Vec<Value> {
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: alarm(),
            property_identifier: property,
            property_array_index: None,
            property_value: encoded(&value),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        self.written(what, service, &body).await
    }

    /// WritePropertyMultiple of AI-3's `writes`, in that order.
    async fn write_multiple(&mut self, what: &str, writes: &[(P, PropertyValue)]) -> Vec<Value> {
        let property =
            |(property, value): &(P, PropertyValue)| bacnet_services::common::BACnetPropertyValue {
                property_identifier: *property,
                property_array_index: None,
                value: encoded(value),
                priority: None,
            };
        let mut body = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: alarm(),
                list_of_properties: writes.iter().map(property).collect(),
            }],
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
        self.written(what, service, &body).await
    }

    /// AI-3 out of service, so a peer may write its Reliability and
    /// Present_Value.
    async fn out_of_service(&mut self) {
        let out_of_service = PropertyValue::Boolean(true);
        let what = "out of service";
        let values = self
            .write_alarm(what, P::OUT_OF_SERVICE, out_of_service)
            .await;
        assert_eq!(values, [real(10.0), flags(OOS)]);
    }
}

#[tokio::test(start_paused = true)]
async fn a_requests_fault_and_its_clear_are_a_row_each() {
    let mut cov = CovParity::start_alarm().await;
    cov.out_of_service().await;
    let reliability = |code| PropertyValue::Enumerated(code);
    let values = cov
        .write_alarm("fault", P::RELIABILITY, reliability(1))
        .await;
    assert_eq!(values, [real(10.0), flags(IN_ALARM | FAULT | OOS)]);
    let values = cov
        .write_alarm("clear", P::RELIABILITY, reliability(0))
        .await;
    assert_eq!(values, [real(10.0), flags(OOS)]);
    cov.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_requests_value_over_the_limit_is_one_row() {
    let mut cov = CovParity::start_alarm().await;
    cov.out_of_service().await;
    let values = cov
        .write_alarm("over", P::PRESENT_VALUE, PropertyValue::Real(90.0))
        .await;
    assert_eq!(values, [real(90.0), flags(IN_ALARM | OOS)]);
    cov.stop().await;
}

/// Two writes of Present_Value in one request keep a row each (#856), the
/// second's carrying the transition it causes.
#[tokio::test(start_paused = true)]
async fn two_writes_in_one_request_keep_a_row_each_both_ways() {
    let real_value = PropertyValue::Real;
    let mut cov = CovParity::start_alarm().await;
    cov.out_of_service().await;
    let writes = [(PV, real_value(90.0)), (PV, real_value(50.0))];
    let values = cov.write_multiple("high, then back", &writes).await;
    assert_eq!(
        values,
        [real(90.0), real(50.0), flags(OOS)],
        "no transition"
    );
    cov.stop().await;

    let mut cov = CovParity::start_alarm().await;
    cov.out_of_service().await;
    let writes = [(PV, real_value(50.0)), (PV, real_value(90.0))];
    let values = cov.write_multiple("then high", &writes).await;
    let rows = [real(50.0), flags(OOS), real(90.0), flags(IN_ALARM | OOS)];
    assert_eq!(values, rows);
    cov.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_local_write_and_its_transition_are_one_row() {
    let mut cov = CovParity::start_alarm().await;
    let change = Change::Input(PropertyValue::Real(90.0));
    let (server, core, frames, now) = cov.both_local("local", alarm(), change).await;
    assert_eq!(server, core, "the result, server then core");
    assert_eq!(cov.same_notifications("local", &frames, now), 1);
    assert_eq!(values(&frames), [real(90.0), flags(IN_ALARM)]);
    cov.stop().await;
}

/// The same write as a WritePropertyMultiple of one: its attempt is captured
/// as it commits, and the capture after the transition takes its place.
#[tokio::test(start_paused = true)]
async fn a_multiple_write_of_one_over_the_limit_is_one_row() {
    let mut cov = CovParity::start_alarm().await;
    cov.out_of_service().await;
    let writes = [(PV, PropertyValue::Real(90.0))];
    let values = cov.write_multiple("one over", &writes).await;
    assert_eq!(values, [real(90.0), flags(IN_ALARM | OOS)]);
    cov.stop().await;
}

/// A write of a property no reference conveys, after the one over the
/// limit, leaves that write's row to take the transition.
#[tokio::test(start_paused = true)]
async fn a_transition_after_an_unconveyed_write_is_one_row() {
    let mut cov = CovParity::start_alarm().await;
    cov.out_of_service().await;
    let writes = [
        (PV, PropertyValue::Real(90.0)),
        (P::DESCRIPTION, PropertyValue::CharacterString("x".into())),
    ];
    let values = cov.write_multiple("over, then description", &writes).await;
    assert_eq!(values, [real(90.0), flags(IN_ALARM | OOS)]);
    cov.stop().await;
}
