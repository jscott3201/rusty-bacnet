//! The [`Device`](crate::Device) writes against the lab device, on the
//! shared test transport: the priority each write sends, what the write
//! policy and the local checks refuse with nothing sent, a relinquish and
//! its read back, and a preview.

use bacnet_encoding::apdu::Apdu;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, RejectReason};
use bacnet_types::primitives::PropertyValue;

use super::read_many_tests::{Script, Service, forwards, scripted};
use super::tests::{LAB, lab_device, lab_on, oid, stop};
use super::*;
use crate::error::OpsError;
use crate::write::{Commandability, Effect, Occupancy, Priority, WritePolicy};

fn p(level: u8) -> Priority {
    Priority::new(level).unwrap()
}

/// The client's send log.
fn log(client: &Client<TestTransport>) -> SendLog {
    client.low_level().transport().sent()
}

/// The confirmed services sent, in order.
fn services(log: &SendLog) -> Vec<ConfirmedServiceChoice> {
    log.unicasts()
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request) => Some(request.service_choice),
            _ => None,
        })
        .collect()
}

/// The WriteProperty requests sent, decoded.
fn writes(log: &SendLog) -> Vec<WritePropertyRequest> {
    log.unicasts()
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request)
                if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY =>
            {
                Some(WritePropertyRequest::decode(&request.service_request).unwrap())
            }
            _ => None,
        })
        .collect()
}

const REAL_50: [u8; 5] = [0x44, 0x42, 0x48, 0, 0];

/// No priority asked: an output's Present_Value goes at 16 and says so, a
/// value object's goes without one (the device takes it at 16 if it is
/// commandable), and a property that isn't commandable goes without one.
#[tokio::test(start_paused = true)]
async fn the_default_priority_follows_the_property() {
    let (client, server) = lab_on(forwards).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    let written = device.write("ao:1 pv", "50", None).await.unwrap();
    assert_eq!(written.priority, Some(Priority::LOWEST));
    assert_eq!(written.commandable, Commandability::Always);
    assert_eq!(
        (written.value.text.as_str(), &written.value.raw[..]),
        ("50", &REAL_50[..])
    );
    let bv = device.write("bv:1 pv", "active", None).await.unwrap();
    assert_eq!(
        (bv.priority, bv.commandable),
        (None, Commandability::Optional)
    );
    // A value object's priority, when given, goes as given.
    let bv8 = device.write("bv:1 pv", "active", Some(p(8))).await.unwrap();
    assert_eq!(bv8.priority, Some(p(8)));
    let oos = device.write("ai:1 oos", "true", None).await.unwrap();
    assert_eq!(
        (oos.priority, oos.commandable),
        (None, Commandability::Never)
    );
    let sent_writes = writes(&sent);
    let shape: Vec<_> = sent_writes
        .iter()
        .map(|w| (w.object_identifier, w.property_identifier, w.priority))
        .collect();
    assert_eq!(
        shape,
        [
            (
                oid(ObjectType::ANALOG_OUTPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                Some(16)
            ),
            (
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None
            ),
            (
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                Some(8)
            ),
            (
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::OUT_OF_SERVICE,
                None
            ),
        ]
    );
    assert_eq!(sent_writes[0].property_value, REAL_50);
    // A typed value goes as it is.
    let typed = device
        .write("ao:1 pv", PropertyValue::Real(50.0), Some(p(12)))
        .await
        .unwrap();
    assert_eq!(
        (typed.priority, &typed.value.raw[..]),
        (Some(p(12)), &REAL_50[..])
    );
    drop(device);
    stop(client, server).await;
}

/// Priorities 1, 2, 5 and 6 are refused by default, for a write and a
/// relinquish, the priority given or named in the text, with nothing
/// sent; allowed on a clone, each goes, and the other clones still refuse.
#[tokio::test(start_paused = true)]
async fn guarded_priorities_are_refused_until_allowed() {
    let (client, server) = lab_on(forwards).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    // An output, which the standard makes commandable, and a value object,
    // which may be: the guard is the same for both.
    let targets = [("ao:1", "50"), ("bv:1", "active")];
    for (object, value) in targets {
        let property = format!("{object} pv");
        for guarded in WritePolicy::GUARDED {
            let refused = device
                .write(property.as_str(), value, Some(guarded))
                .await
                .unwrap_err();
            assert!(
                matches!(&refused, OpsError::Denied(denied) if denied.priority == guarded && !denied.relinquish),
                "{property}: {refused:?}"
            );
            let text = format!("{value}@{guarded}");
            let refused = device
                .write(property.as_str(), text.as_str(), None)
                .await
                .unwrap_err();
            assert!(matches!(refused, OpsError::Denied(_)), "{refused:?}");
            let refused = device.relinquish(object, guarded).await.unwrap_err();
            assert!(
                matches!(&refused, OpsError::Denied(denied) if denied.relinquish),
                "{object}: {refused:?}"
            );
        }
    }
    assert!(
        services(&sent).is_empty(),
        "nothing sent: {:?}",
        services(&sent)
    );
    for (object, value) in targets {
        let property = format!("{object} pv");
        for guarded in WritePolicy::GUARDED {
            let allowed =
                client.with_write_policy(WritePolicy::default().allow_priorities([guarded]));
            let opened = allowed.device(LAB).await.unwrap();
            // The device may refuse priority 6 itself; it was sent either way.
            let _ = opened.write(property.as_str(), value, Some(guarded)).await;
            assert_eq!(
                writes(&sent).last().map(|w| w.priority),
                Some(Some(guarded.get()))
            );
            assert!(matches!(
                device.write(property.as_str(), value, Some(guarded)).await,
                Err(OpsError::Denied(_))
            ));
        }
    }
    assert_eq!(writes(&sent).len(), 2 * WritePolicy::GUARDED.len());
    drop(device);
    stop(client, server).await;
}

/// The text's priority and the call's must agree: two different ones are
/// refused with nothing sent, never one picked.
#[tokio::test(start_paused = true)]
async fn the_text_and_the_call_name_one_priority() {
    let (client, server) = lab_on(forwards).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    let refused = device
        .write("ao:1 pv", "72.5@8", Some(p(10)))
        .await
        .unwrap_err();
    assert!(
        matches!(
            &refused,
            OpsError::Parse {
                what: "priority",
                ..
            }
        ),
        "{refused:?}"
    );
    assert!(services(&sent).is_empty());
    let same = device.write("ao:1 pv", "72.5@8", Some(p(8))).await.unwrap();
    let text = device.write("ao:1 pv", "72.5@9", None).await.unwrap();
    assert_eq!((same.priority, text.priority), (Some(p(8)), Some(p(9))));
    let priorities: Vec<_> = writes(&sent).iter().map(|w| w.priority).collect();
    assert_eq!(priorities, [Some(8), Some(9)]);
    drop(device);
    stop(client, server).await;
}

/// What is refused before anything is sent: a priority on a property that
/// takes none, NULL where it would empty a slot, a write to Priority_Array
/// (an element is not a priority), and a relinquish or priority read of an
/// object with no Priority_Array.
#[tokio::test(start_paused = true)]
async fn refusals_send_nothing() {
    let (client, server) = lab_on(forwards).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    let what = |error: OpsError| match error {
        OpsError::Parse { what, .. } => what,
        other => panic!("{other:?}"),
    };
    let priority = device
        .write("ai:1 oos", "true", Some(p(8)))
        .await
        .unwrap_err();
    assert_eq!(what(priority), "priority");
    // Raw octets holding NULL are refused as a typed NULL is.
    let raw = PropertyValue::ApplicationData(vec![0x00]);
    let raw_null = device.write("ao:1 pv", raw, Some(p(8))).await.unwrap_err();
    assert_eq!(what(raw_null), "value");
    let raw_list = PropertyValue::List(vec![PropertyValue::Null]);
    let raw_null = device
        .write("bv:1 pv", raw_list, Some(p(8)))
        .await
        .unwrap_err();
    assert_eq!(what(raw_null), "value");
    let null = device
        .write("ao:1 pv", "null", Some(p(8)))
        .await
        .unwrap_err();
    assert!(null.to_string().contains("relinquish"), "{null}");
    assert_eq!(what(null), "value");
    for slot in ["ao:1 pa[8]", "ao:1 priority-array"] {
        let refused = device.write(slot, "5", None).await.unwrap_err();
        assert!(refused.to_string().contains("read-only"), "{refused}");
        assert_eq!(what(refused), "property");
    }
    assert_eq!(
        what(device.relinquish("ai:1", p(8)).await.unwrap_err()),
        "object"
    );
    assert_eq!(
        what(device.priority_array("ai:1").await.unwrap_err()),
        "object"
    );
    let channel = oid(ObjectType::CHANNEL, 1);
    assert_eq!(
        what(device.relinquish(channel, p(8)).await.unwrap_err()),
        "object"
    );
    assert_eq!(
        what(device.write("ao:1 pv", "50@17", None).await.unwrap_err()),
        "value"
    );
    assert!(services(&sent).is_empty(), "{:?}", services(&sent));
    drop(device);
    stop(client, server).await;
}

/// A relinquish writes NULL at its slot and reads the priorities back in
/// one RPM: two requests, and the next slot down takes control.
#[tokio::test(start_paused = true)]
async fn a_relinquish_empties_its_slot_and_reads_back_once() {
    let (client, server) = lab_on(forwards).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    device.write("ao:1 pv", "30", Some(p(9))).await.unwrap();
    device.write("ao:1 pv", "40", Some(p(12))).await.unwrap();
    let view = device.priority_array("ao:1").await.unwrap();
    assert_eq!(view.in_control, Some(p(9)));
    assert_eq!(view.slot(p(9)).map(|v| v.text.as_str()), Some("30"));
    assert_eq!(view.present_value.text, "30");
    assert_eq!(view.slots.iter().filter(|slot| slot.is_some()).count(), 2);
    sent.clear();
    let after = device.relinquish("ao:1", p(9)).await.unwrap();
    assert_eq!(
        services(&sent),
        [
            ConfirmedServiceChoice::WRITE_PROPERTY,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
        ]
    );
    let null = &writes(&sent)[0];
    assert_eq!(
        (null.priority, &null.property_value[..]),
        (Some(9), &[0x00][..])
    );
    assert_eq!(after.in_control, Some(p(12)));
    assert_eq!(after.slot(p(9)), None);
    assert_eq!(after.present_value.text, "40");
    // The last one out leaves Relinquish_Default in control.
    let last = device.relinquish("ao:1", p(12)).await.unwrap();
    assert_eq!(last.in_control, None);
    let default = last.relinquish_default.as_ref().unwrap();
    assert_eq!(last.present_value.text, default.text);
    drop(device);
    stop(client, server).await;
}

fn rpm_silent(inner: bacnet_transport::loopback::LoopbackTransport) -> TestTransport {
    scripted(inner, |service, _, _| match service {
        Service::Rpm => Script::Silent,
        Service::Rp => Script::Forward,
    })
}

/// A relinquish the device took whose read back times out says it went,
/// so nothing invites sending it again, and sends it once.
#[tokio::test(start_paused = true)]
async fn a_relinquish_whose_read_back_fails_says_it_went() {
    let (client, server) = lab_on(rpm_silent).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    let error = device.relinquish("ao:1", p(9)).await.unwrap_err();
    let OpsError::ReadBack { written, source } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(written.priority, Some(p(9)));
    assert!(written.value.is_null());
    assert!(source.timed_out(), "{source:?}");
    assert!(!error.unreachable() && !error.timed_out());
    assert!(error.to_string().contains("acknowledged"), "{error}");
    assert_eq!(writes(&sent).len(), 1);
    drop(device);
    stop(client, server).await;
}

fn rpm_refused(inner: bacnet_transport::loopback::LoopbackTransport) -> TestTransport {
    scripted(inner, |service, _, _| match service {
        Service::Rpm => Script::Reject(RejectReason::UNRECOGNIZED_SERVICE),
        Service::Rp => Script::Forward,
    })
}

/// A device that refuses RPM reads its priorities back by ReadProperty.
#[tokio::test(start_paused = true)]
async fn a_read_back_falls_back_to_read_property() {
    let (client, server) = lab_on(rpm_refused).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    device.write("ao:1 pv", "30", Some(p(9))).await.unwrap();
    let after = device.relinquish("ao:1", p(9)).await.unwrap();
    assert_eq!(after.in_control, None);
    let reads = services(&sent)
        .into_iter()
        .filter(|s| *s == ConfirmedServiceChoice::READ_PROPERTY)
        .count();
    assert_eq!(reads, 3);
    drop(device);
    stop(client, server).await;
}

/// A preview reads once and sends nothing: the slot is empty but a
/// stronger one keeps control, then held by another command, and a write
/// at a stronger priority would change Present_Value; the policy's refusal
/// is reported, not raised.
#[tokio::test(start_paused = true)]
async fn a_preview_reads_the_slots_and_sends_nothing() {
    let (client, server) = lab_on(forwards).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    device.write("ao:1 pv", "20", Some(p(4))).await.unwrap();
    sent.clear();
    let weaker = device
        .preview_write("ao:1 pv", "50", Some(p(8)))
        .await
        .unwrap();
    assert_eq!(
        services(&sent),
        [ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE]
    );
    assert_eq!(weaker.priority, Some(p(8)));
    assert_eq!(weaker.occupancy, Occupancy::Empty);
    assert_eq!(weaker.before, None);
    assert_eq!(
        weaker.effect,
        Effect::Unchanged {
            held_by: Some(p(4))
        }
    );
    assert_eq!(
        weaker.priorities.as_ref().and_then(|v| v.in_control),
        Some(p(4))
    );
    assert_eq!(weaker.denied, None);
    let held = device
        .preview_write("ao:1 pv", "50", Some(p(4)))
        .await
        .unwrap();
    assert_eq!(held.occupancy, Occupancy::Held);
    assert_eq!(held.before.as_ref().map(|v| v.text.as_str()), Some("20"));
    let stronger = device
        .preview_write("ao:1 pv", "50", Some(p(3)))
        .await
        .unwrap();
    assert_eq!(
        stronger.effect,
        Effect::Changes {
            from: "20".into(),
            to: "50".into()
        }
    );
    let guarded = device
        .preview_write("ao:1 pv", "50", Some(p(1)))
        .await
        .unwrap();
    assert_eq!(guarded.denied.map(|d| d.priority), Some(p(1)));
    // No priority: 16, as the write would send.
    let default = device.preview_write("ao:1 pv", "50", None).await.unwrap();
    assert_eq!(default.priority, Some(Priority::LOWEST));
    // A value object with a Priority_Array is commandable by the read.
    let bv = device
        .preview_write("bv:1 pv", "active", None)
        .await
        .unwrap();
    assert_eq!(
        (bv.priority, bv.occupancy),
        (Some(Priority::LOWEST), Occupancy::Empty)
    );
    // A property that isn't commandable: one ReadProperty of it.
    sent.clear();
    let oos = device
        .preview_write("ai:1 oos", "true", None)
        .await
        .unwrap();
    assert_eq!(services(&sent), [ConfirmedServiceChoice::READ_PROPERTY]);
    assert_eq!(
        (oos.priority, oos.occupancy),
        (None, Occupancy::NotCommandable)
    );
    assert_eq!(oos.before.as_ref().map(|v| v.text.as_str()), Some("false"));
    assert_eq!(oos.effect, Effect::NotPresentValue);
    assert!(writes(&sent).is_empty());
    drop(device);
    stop(client, server).await;
}

/// An RPM answer saying the object has no Priority_Array or
/// Relinquish_Default, with its Present_Value inactive: a value object
/// built without priorities.
fn no_priority_array() -> Vec<u8> {
    use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let unknown = |property| ReadResultElement {
        property_identifier: property,
        property_array_index: None,
        property_value: None,
        error: Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
    };
    let ack = ReadPropertyMultipleACK {
        list_of_read_access_results: vec![ReadAccessResult {
            object_identifier: oid(ObjectType::BINARY_VALUE, 1),
            list_of_results: vec![
                unknown(PropertyIdentifier::PRIORITY_ARRAY),
                unknown(PropertyIdentifier::RELINQUISH_DEFAULT),
                ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(vec![0x91, 0x00]),
                    error: None,
                },
            ],
        }],
    };
    let mut data = bytes::BytesMut::new();
    ack.encode(&mut data).unwrap();
    data.to_vec()
}

fn without_priorities(inner: bacnet_transport::loopback::LoopbackTransport) -> TestTransport {
    scripted(inner, |service, _, _| match service {
        Service::Rpm => Script::Ack(no_priority_array()),
        Service::Rp => Script::Forward,
    })
}

/// A relinquish the device takes as a no-op, because the object has no
/// Priority_Array, says so, and says the NULL changed nothing; it went
/// once. A read of the priorities says the same, and a preview finds the
/// value object not commandable, while an output whose Priority_Array
/// isn't given is previewed as the write would go, at 16, slots unknown.
#[tokio::test(start_paused = true)]
async fn an_object_without_a_priority_array_says_so() {
    let (client, server) = lab_on(without_priorities).await;
    let sent = log(&client);
    let device = lab_device(&client).await;
    let error = device.relinquish("bv:1", p(9)).await.unwrap_err();
    let OpsError::NoPriorityArray {
        object,
        relinquished: Some(written),
        ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(*object, oid(ObjectType::BINARY_VALUE, 1));
    assert_eq!(written.priority, Some(p(9)));
    assert_eq!(error.written().map(|w| w.priority), Some(Some(p(9))));
    let text = error.to_string();
    assert!(
        text.contains("no Priority_Array") && text.contains("changed nothing"),
        "{text}"
    );
    assert_eq!(writes(&sent).len(), 1);
    let read = device.priority_array("bv:1").await.unwrap_err();
    assert!(
        matches!(
            read,
            OpsError::NoPriorityArray {
                relinquished: None,
                ..
            }
        ),
        "{read:?}"
    );
    assert!(read.written().is_none());
    let bv = device
        .preview_write("bv:1 pv", "active", Some(p(8)))
        .await
        .unwrap();
    assert_eq!(
        (bv.priority, bv.occupancy),
        (None, Occupancy::NotCommandable)
    );
    assert_eq!(bv.before.map(|v| v.text), Some("inactive".into()));
    let ao = device.preview_write("ao:1 pv", "50", None).await.unwrap();
    assert_eq!(
        (ao.priority, ao.occupancy, ao.effect),
        (Some(Priority::LOWEST), Occupancy::Unknown, Effect::Unknown)
    );
    assert_eq!(writes(&sent).len(), 1, "previews send nothing");
    drop(device);
    stop(client, server).await;
}

/// An element of a value object's Present_Value is no vendor's
/// commandable property: its preview reads it and finds no slot.
#[tokio::test(start_paused = true)]
async fn an_indexed_present_value_preview_is_not_a_vendor_property() {
    let (client, server) = lab_on(forwards).await;
    let device = lab_device(&client).await;
    let indexed = device
        .preview_write("bv:1 pv[1]", "active", None)
        .await
        .unwrap();
    assert_eq!(indexed.occupancy, Occupancy::NotCommandable);
    assert_eq!(indexed.effect, Effect::NotPresentValue);
    drop(device);
    stop(client, server).await;
}
