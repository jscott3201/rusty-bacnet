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

/// One answer per device, object and source (#2596): the same I-Have heard
/// twice is kept once, but two devices sharing an instance at two
/// addresses both come back. `keep` refuses what the caller doesn't want,
/// and an answer to another question is never kept.
#[tokio::test(start_paused = true)]
async fn answers_are_kept_once_per_device_object_and_source() {
    let (tx, mut rx) = broadcast::channel(16);
    let from = |mac: u8, name: &str| ReceivedIHave {
        source_mac: [mac].as_slice().into(),
        ..reply(40, name)
    };
    for sent in [
        from(5, "Zone"),
        from(5, "Zone"),
        from(6, "Zone"),
        from(7, "Zone"),
        from(8, "Other"),
    ] {
        tx.send(sent).unwrap();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let name = WhoHasObject::Name("Zone".into());
    let kept = collect_answers(&mut rx, deadline, &name, None, |reply| {
        reply.source_mac.as_slice() != [7]
    })
    .await;
    let macs: Vec<&[u8]> = kept.iter().map(|r| r.source_mac.as_slice()).collect();
    assert_eq!(macs, [[5].as_slice(), &[6]]);
}

/// Collection stops at the cap, before the wait is over.
#[tokio::test(start_paused = true)]
async fn collection_stops_at_the_cap() {
    let (tx, mut rx) = broadcast::channel(MAX_I_HAVE_REPLIES + 8);
    for device in 0..MAX_I_HAVE_REPLIES as u32 + 4 {
        tx.send(reply(device, "Zone")).unwrap();
    }
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs(5);
    let name = WhoHasObject::Name("Zone".into());
    let kept = collect_answers(&mut rx, deadline, &name, None, |_| true).await;
    assert_eq!(kept.len(), MAX_I_HAVE_REPLIES);
    assert_eq!(
        tokio::time::Instant::now(),
        start,
        "it didn't wait out the deadline"
    );
}

/// An answer `keep` refuses doesn't count toward the cap: past the cap's
/// worth of refused ones, the answers kept still come.
#[tokio::test(start_paused = true)]
async fn refused_answers_do_not_count_toward_the_cap() {
    let (tx, mut rx) = broadcast::channel(MAX_I_HAVE_REPLIES + 8);
    for device in 0..MAX_I_HAVE_REPLIES as u32 + 3 {
        tx.send(reply(device, "Zone")).unwrap();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let name = WhoHasObject::Name("Zone".into());
    let wanted = MAX_I_HAVE_REPLIES as u32;
    let kept = collect_answers(&mut rx, deadline, &name, None, |reply| {
        reply.i_have.device_identifier.instance_number() >= wanted
    })
    .await;
    let devices: Vec<u32> = kept
        .iter()
        .map(|r| r.i_have.device_identifier.instance_number())
        .collect();
    assert_eq!(devices, [wanted, wanted + 1, wanted + 2]);
}
