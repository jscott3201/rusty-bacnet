//! The node's Device lists what its database holds (#2669): a peer's
//! CreateObject and DeleteObject through the node's full server, and the
//! application's own add, move its Object_List and Database_Revision.
use super::*;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_services::object_mgmt::ObjectSpecifier;

fn node_device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap()
}

/// The node Device's `property` at `index`, as the standalone client reads
/// it, decoded.
async fn read(lab: &Lab, property: PropertyIdentifier, index: Option<u32>) -> Vec<PropertyValue> {
    let ack = lab
        .other
        .read_property(&Wire::mac(NODE), node_device(), property, index)
        .await
        .unwrap();
    let bytes = ack.property_value;
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, next) = decode_application_value(&bytes, offset).unwrap();
        values.push(value);
        offset = next;
    }
    values
}

/// The node Device's Object_List, whole, and its count and last entry
/// read by index, which must agree with it, with its Database_Revision.
async fn listed(lab: &Lab) -> (Vec<ObjectIdentifier>, u64) {
    let list: Vec<_> = read(lab, PropertyIdentifier::OBJECT_LIST, None)
        .await
        .into_iter()
        .map(|entry| match entry {
            PropertyValue::ObjectIdentifier(oid) => oid,
            other => panic!("Object_List holds {other:?}"),
        })
        .collect();
    let count = list.len() as u32;
    assert_eq!(
        read(lab, PropertyIdentifier::OBJECT_LIST, Some(0)).await,
        [PropertyValue::Unsigned(u64::from(count))]
    );
    assert_eq!(
        read(lab, PropertyIdentifier::OBJECT_LIST, Some(count)).await,
        [PropertyValue::ObjectIdentifier(list[list.len() - 1])]
    );
    let [PropertyValue::Unsigned(revision)] =
        read(lab, PropertyIdentifier::DATABASE_REVISION, None).await[..]
    else {
        panic!("Database_Revision is one Unsigned");
    };
    (list, revision)
}

#[tokio::test(start_paused = true)]
async fn the_nodes_object_list_follows_creates_deletes_and_the_application() {
    let lab = lab().await;
    let node = Wire::mac(NODE);
    let (built, revision) = listed(&lab).await;
    assert_eq!(built[..2], [node_device(), ai1()]);

    let created = lab
        .other
        .create_object(
            &node,
            ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            vec![],
        )
        .await
        .unwrap();
    let mut expected = built.clone();
    expected.push(created);
    assert_eq!(listed(&lab).await, (expected.clone(), revision + 1));

    lab.other.delete_object(&node, ai1()).await.unwrap();
    expected.retain(|oid| *oid != ai1());
    assert_eq!(listed(&lab).await, (expected.clone(), revision + 2));

    let bv3 = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap();
    lab.server()
        .database()
        .write()
        .await
        .add(Box::new(BinaryValueObject::new(3, "BV-3").unwrap()))
        .unwrap();
    expected.push(bv3);
    assert_eq!(listed(&lab).await, (expected, revision + 3));

    stop(lab).await;
}
