use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn reply(device: u32, name: &str) -> ReceivedIHave {
    ReceivedIHave {
        i_have: IHaveRequest {
            device_identifier: oid(ObjectType::DEVICE, device),
            object_identifier: oid(ObjectType::ANALOG_INPUT, 3),
            object_name: name.into(),
        },
        source_mac: [5].as_slice().into(),
        source_network: None,
    }
}

#[test]
fn an_i_have_answers_only_its_own_question() {
    let got = reply(40, "Supply Air Temp");
    let name = |n: &str| WhoHasObject::Name(n.into());
    assert!(got.answers(&name("Supply Air Temp"), None));
    // Names compare exactly.
    assert!(!got.answers(&name("supply air temp"), None));
    assert!(!got.answers(&name("Supply Air"), None));
    let id = |instance| WhoHasObject::Identifier(oid(ObjectType::ANALOG_INPUT, instance));
    assert!(got.answers(&id(3), None));
    assert!(!got.answers(&id(4), None));
    // The device's instance has to lie in the range asked.
    let range = |low, high| Some(DeviceInstanceRange::new(low, high).unwrap());
    assert!(got.answers(&id(3), range(40, 40)));
    assert!(got.answers(&id(3), range(1, 100)));
    assert!(!got.answers(&id(3), range(41, 100)));
    assert!(!got.answers(&id(3), range(1, 39)));
}

/// A collector that falls behind loses the oldest, is told, and goes on
/// with the rest until the deadline.
#[tokio::test(start_paused = true)]
async fn a_lagging_collector_keeps_the_newest() {
    let (tx, mut rx) = broadcast::channel(2);
    for n in 0..5u32 {
        tx.send(n).unwrap();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let mut seen = Vec::new();
    collect_until(&mut rx, deadline, "test", |n| seen.push(n)).await;
    assert_eq!(seen, [3, 4]);
    assert_eq!(tokio::time::Instant::now(), deadline);
}

/// With nobody subscribed, an I-Have is not decoded, so a malformed one
/// costs nothing; a subscriber gets the decoded one. A malformed one with a
/// subscriber warns through the client's decode throttle (#1950).
#[tokio::test(start_paused = true)]
async fn delivery_reaches_subscribers_only() {
    let (tx, _) = broadcast::channel(4);
    let throttle = WarnThrottle::default();
    let mut encoded = BytesMut::new();
    reply(40, "Supply Air Temp")
        .i_have
        .encode(&mut encoded)
        .unwrap();
    deliver(&tx, &[5], &None, &[0xFF], &throttle);
    deliver(&tx, &[5], &None, &encoded, &throttle);
    let mut rx = tx.subscribe();
    deliver(&tx, &[5], &None, &[0xFF], &throttle);
    assert!(rx.try_recv().is_err());
    deliver(&tx, &[5], &None, &encoded, &throttle);
    assert_eq!(rx.try_recv().unwrap(), reply(40, "Supply Air Temp"));
    assert_eq!(
        throttle.note(),
        None,
        "the malformed one with a subscriber warned, and only it"
    );
    tokio::time::advance(bacnet_transport::warn_throttle::WARNING_INTERVAL).await;
    assert_eq!(throttle.note(), Some(2), "the check above, and this one");
}
