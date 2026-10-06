//! A Group's Present_Value through the running server (Clause 12.14.6). A
//! member that names the Device's COV subscription lists reads the request's
//! live snapshot (#1171), and the member rows count against the request's
//! work limit, so a read that would pass it gets the abort any
//! ReadPropertyMultiple over its work budget gets (#1172).
use super::*;
use bacnet_objects::group::GroupObject;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadResultElement};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};

const ALL: PropertyIdentifier = PropertyIdentifier::ALL;
const STATUS_FLAGS: PropertyIdentifier = PropertyIdentifier::STATUS_FLAGS;
const DESCRIPTION: PropertyIdentifier = PropertyIdentifier::DESCRIPTION;
const OBJECT_NAME: PropertyIdentifier = PropertyIdentifier::OBJECT_NAME;

/// One member: an object and the properties the Group reports from it.
pub(super) type Member = (ObjectIdentifier, &'static [PropertyIdentifier]);

pub(super) fn group(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::GROUP, instance).unwrap()
}

pub(super) async fn add_group(wire: &Wire, instance: u32, members: &[Member]) {
    let mut object = GroupObject::new(instance, format!("GRP-{instance}")).unwrap();
    for &(object_identifier, properties) in members {
        object
            .add_member(ReadAccessSpecification {
                object_identifier,
                list_of_property_references: properties
                    .iter()
                    .map(|&property_identifier| PropertyReference {
                        property_identifier,
                        property_array_index: None,
                    })
                    .collect(),
            })
            .unwrap();
    }
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(object))
        .unwrap();
}

pub(super) fn read_property(object: ObjectIdentifier) -> (ConfirmedServiceChoice, BytesMut) {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: PV,
        property_array_index: None,
    }
    .encode(&mut request);
    (ConfirmedServiceChoice::READ_PROPERTY, request)
}

/// ReadRange of a Group's whole Present_Value, every item.
pub(super) fn read_range(object: ObjectIdentifier) -> (ConfirmedServiceChoice, BytesMut) {
    let mut request = BytesMut::new();
    ReadRangeRequest {
        object_identifier: object,
        property_identifier: PV,
        property_array_index: None,
        range: None,
    }
    .encode(&mut request)
    .unwrap();
    (ConfirmedServiceChoice::READ_RANGE, request)
}

/// Every member row of a Present_Value, across its ReadAccessResult elements.
fn member_rows(present_value: &[u8]) -> Vec<ReadResultElement> {
    ReadPropertyMultipleACK::decode(present_value)
        .unwrap()
        .list_of_read_access_results
        .into_iter()
        .flat_map(|result| result.list_of_results)
        .collect()
}

/// The live lists as the Device itself serves them.
struct Lists {
    active: Vec<BACnetCOVSubscription>,
    multiple: Vec<BACnetCOVMultipleSubscription>,
}

/// One SubscribeCOV with no lifetime (so its entry has no countdown) and one
/// SubscribeCOVPropertyMultiple, then both lists read from the Device.
async fn subscribe(wire: &mut Wire) -> Lists {
    simple_ack(
        wire.send(&direct(), subscribe_cov(20, av(1), Some(false), None))
            .await,
    );
    simple_ack(
        wire.send(
            &direct(),
            subscribe_cov_property_multiple(
                21,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)])],
            ),
        )
        .await,
    );
    let lists = Lists {
        active: wire.active().await,
        multiple: untimed(&wire.multiple().await, &[300]),
    };
    assert_eq!((lists.active.len(), lists.multiple.len()), (1, 1));
    lists
}

/// Each of `expected` has a member row carrying that live list.
fn assert_live(rows: &[ReadResultElement], lists: &Lists, expected: &[PropertyIdentifier]) {
    for &property in expected {
        let row = rows
            .iter()
            .find(|row| row.property_identifier == property)
            .unwrap_or_else(|| panic!("no {property:?} member row in {rows:?}"));
        let value = row.property_value.as_deref().expect("a list value");
        if property == ACTIVE {
            assert_eq!(decode_subscriptions(value), lists.active);
        } else {
            assert_eq!(untimed(&decode_contexts(value), &[300]), lists.multiple);
        }
    }
}

/// Groups 1 to 3, each with one member: one naming each list on the Device,
/// and one reading ALL through the Device wildcard. The last item is the
/// lists that member reads.
fn cases() -> [(u32, Member, &'static [PropertyIdentifier]); 3] {
    [
        (1, (device(), &[ACTIVE]), &[ACTIVE]),
        (2, (device(), &[MULTIPLE]), &[MULTIPLE]),
        (3, (wildcard(), &[ALL]), &[ACTIVE, MULTIPLE]),
    ]
}

#[tokio::test(start_paused = true)]
async fn group_members_read_the_live_device_cov_lists_through_rp_readrange_and_local_reads() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let lists = subscribe(&mut wire).await;
    for (instance, member, expected) in cases() {
        add_group(&wire, instance, &[member]).await;
        let value = wire.read(group(instance), PV, None).await.unwrap();
        assert_live(&member_rows(&value), &lists, expected);
        // ReadRange and the local read plan and sample the same way.
        let Apdu::ComplexAck(ack) = wire.send(&direct(), read_range(group(instance))).await else {
            panic!("ReadRange of Group {instance}");
        };
        let items = ReadRangeAck::decode(&ack.service_ack).unwrap().item_data;
        assert_live(&member_rows(&items), &lists, expected);
        let PropertyValue::List(local) = wire
            .server
            .read_local(&group(instance), PV, None)
            .await
            .unwrap()
        else {
            panic!("Present_Value is a list");
        };
        let local: Vec<u8> = local
            .iter()
            .flat_map(|element| match element {
                PropertyValue::ApplicationData(bytes) => bytes.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_live(&member_rows(&local), &lists, expected);
    }
    wire.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn group_members_read_the_live_device_cov_lists_through_rpm() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let lists = subscribe(&mut wire).await;
    for (instance, member, expected) in cases() {
        add_group(&wire, instance, &[member]).await;
        // By name and through ALL on the Group.
        for selector in [PV, ALL] {
            let ack = wire
                .rpm(vec![(group(instance), vec![(selector, None)])])
                .await;
            let present_value = rows(&ack, PV);
            assert_eq!(present_value.len(), 1, "{selector:?}");
            assert_live(&member_rows(&present_value[0]), &lists, expected);
        }
    }
    // A request whose own Device row takes only Active_COV_Subscriptions
    // still snapshots the Multiple list a member reads.
    let ack = wire
        .rpm(vec![
            (device(), vec![(ACTIVE, None)]),
            (group(2), vec![(PV, None)]),
        ])
        .await;
    assert_eq!(decode_subscriptions(&rows(&ack, ACTIVE)[0]), lists.active);
    assert_live(&member_rows(&rows(&ack, PV)[0]), &lists, &[MULTIPLE]);
    wire.server.stop().await.unwrap();
}

pub(super) fn abort(response: Apdu) -> (bool, AbortReason) {
    match response {
        Apdu::Abort(abort) => (abort.sent_by_server, abort.abort_reason),
        other => panic!("expected an abort, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn group_present_value_past_the_work_limit_aborts_as_any_rpm_does() {
    let mut wire = Wire::start(ServerConfig {
        read_property_multiple_budget: ReadPropertyMultipleBudget {
            max_result_elements: 8,
            ..Default::default()
        },
        ..Default::default()
    })
    .await;
    let four: &[PropertyIdentifier] = &[PV, STATUS_FLAGS, DESCRIPTION, OBJECT_NAME];
    // Group 7 has eight member rows: with its own row, nine.
    add_group(&wire, 7, &[(av(1), four), (av(2), four)]).await;
    // Groups 8 and 9 fit alone, in four and five rows, but not together.
    add_group(&wire, 8, &[(av(1), &four[..3])]).await;
    add_group(&wire, 9, &[(av(2), four)]).await;

    let ordinary = abort(
        wire.send(&direct(), rpm_request(vec![(av(1), vec![(PV, None); 9])]))
            .await,
    );
    assert_eq!(ordinary, (true, AbortReason::OUT_OF_RESOURCES));
    // Without its members ALL on Group 7 would be six rows and fit.
    for selector in [PV, ALL] {
        let request = rpm_request(vec![(group(7), vec![(selector, None)])]);
        assert_eq!(abort(wire.send(&direct(), request).await), ordinary);
    }
    // ReadProperty and ReadRange get the limit of an RPM naming only it.
    assert_eq!(
        abort(wire.send(&direct(), read_property(group(7))).await),
        ordinary
    );
    assert_eq!(
        abort(wire.send(&direct(), read_range(group(7))).await),
        ordinary
    );
    assert!(matches!(
        wire.server.read_local(&group(7), PV, None).await,
        Err(Error::Abort { reason }) if reason == AbortReason::OUT_OF_RESOURCES.to_raw()
    ));

    for (instance, member_count) in [(8, 3), (9, 4)] {
        let ack = wire.rpm(vec![(group(instance), vec![(PV, None)])]).await;
        assert_eq!(member_rows(&rows(&ack, PV)[0]).len(), member_count);
        assert!(wire.read(group(instance), PV, None).await.is_ok());
    }
    let both = rpm_request(vec![
        (group(8), vec![(PV, None)]),
        (group(9), vec![(PV, None)]),
    ]);
    assert_eq!(abort(wire.send(&direct(), both).await), ordinary);
    wire.server.stop().await.unwrap();
}
