//! A Lighting Output's Present_Value between 0.0 and 1.0 is taken as 1.0
//! (#1385, Clause 12.54.4) through `BACnetServer::write_local` as over
//! WriteProperty, and the SubscribeCOV report carries the stored 1.0.
use super::command_action_wire_tests::read_db;
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::lighting::LightingOutputObject;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::error::RemoteError;

fn lo1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, 1).unwrap()
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

async fn write_local(h: &Harness, value: PropertyValue) -> Result<(), Error> {
    h.server
        .write_local(
            &lo1(),
            PV,
            None,
            value,
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

/// Present_Value, Priority_Array[8] and Tracking_Value in the database.
async fn levels_at_8(h: &Harness) -> [PropertyValue; 3] {
    [
        read_db(h, lo1(), PV, None).await,
        read_db(h, lo1(), PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        read_db(h, lo1(), PropertyIdentifier::TRACKING_VALUE, None).await,
    ]
}

/// The next SubscribeCOV notification's Present_Value, as encoded.
async fn reported(h: &Harness) -> Vec<u8> {
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, lo1());
    assert_eq!(notification.list_of_values[0].property_identifier, PV);
    notification.list_of_values[0].value.clone()
}

#[tokio::test(start_paused = true)]
async fn lighting_output_local_write_below_one_percent_is_stored_and_reported_as_one_percent() {
    use PropertyValue::{Null, Real};
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(LightingOutputObject::new(1, "LO-1").unwrap()))
            .unwrap();
    })
    .await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1385,
        monitored_object_identifier: lo1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    assert_eq!(reported(&h).await, encoded(&Real(0.0)));

    for level in [f32::from_bits(1), 0.5, 1.0f32.next_down()] {
        write_local(&h, Real(level)).await.unwrap();
        assert_eq!(levels_at_8(&h).await, [Real(1.0), Real(1.0), Real(1.0)]);
        assert_eq!(reported(&h).await, encoded(&Real(1.0)), "{level:e}");
        write_local(&h, Null).await.unwrap();
        assert_eq!(reported(&h).await, encoded(&Real(0.0)));
    }

    // 1.0 itself is stored as written, and no further report goes out for a
    // write that lands on the same level.
    write_local(&h, Real(1.0)).await.unwrap();
    assert_eq!(reported(&h).await, encoded(&Real(1.0)));
    write_local(&h, Real(0.25)).await.unwrap();
    h.no_notification().await;

    for level in [-1.5, 100.5, f32::NAN] {
        match write_local(&h, Real(level)).await {
            Err(Error::Remote(RemoteError::Error { class, code })) => {
                assert_eq!(class, ErrorClass::PROPERTY);
                assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE);
            }
            other => panic!("{level:e}: expected VALUE_OUT_OF_RANGE, got {other:?}"),
        }
    }
    assert_eq!(levels_at_8(&h).await, [Real(1.0), Real(1.0), Real(1.0)]);
    h.no_notification().await;

    // The same level over WriteProperty, at the same slot.
    write_local(&h, Null).await.unwrap();
    assert_eq!(reported(&h).await, encoded(&Real(0.0)));
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: lo1(),
        property_identifier: PV,
        property_array_index: None,
        property_value: encoded(&Real(0.5)),
        priority: Some(8),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(reported(&h).await, encoded(&Real(1.0)));
    assert_eq!(levels_at_8(&h).await, [Real(1.0), Real(1.0), Real(1.0)]);
    h.server.stop().await.unwrap();
}
