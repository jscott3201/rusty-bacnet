use super::*;

/// AddListElement and RemoveListElement end to end over loopback UDP (#1026,
/// #1027): a present element is left as it is, an absent one refuses the
/// removal and keeps the list, and every refusal reaches the client as a
/// ChangeList-Error with the First Failed Element Number.
#[tokio::test]
async fn list_element_semantics_and_change_list_errors_reach_the_client() {
    use bacnet_objects::multistate::MultiStateInputObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::{Error, ErrorDetail};
    use bacnet_types::primitives::PropertyValue;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();
    let msi = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
    let mut object = MultiStateInputObject::new(1, "Mode", 3).unwrap();
    object.set_alarm_values(vec![1]);
    server
        .database()
        .write()
        .await
        .add(Box::new(object))
        .unwrap();
    let alarm_values = |values: &[u64]| {
        PropertyValue::List(
            values
                .iter()
                .copied()
                .map(PropertyValue::Unsigned)
                .collect(),
        )
    };
    let read = || async {
        server
            .database()
            .read()
            .await
            .get(&msi)
            .unwrap()
            .read_property(PropertyIdentifier::ALARM_VALUES, None)
            .unwrap()
    };
    let refusal = |result: Result<(), Error>| match result {
        Err(Error::Structured {
            class,
            code,
            detail,
        }) => match *detail {
            ErrorDetail::FirstFailedElementNumber(element) => (class, code, element),
            other => panic!("expected an element number, got {other:?}"),
        },
        other => panic!("expected a ChangeList-Error, got {other:?}"),
    };

    // 1 is present, so the list gains only 2.
    client
        .add_list_element(
            &server_mac,
            msi,
            PropertyIdentifier::ALARM_VALUES,
            None,
            vec![0x21, 1, 0x21, 2],
        )
        .await
        .unwrap();
    assert_eq!(read().await, alarm_values(&[1, 2]));

    // 3 is absent: the request fails at the second element and 2 stays.
    let result = client
        .remove_list_element(
            &server_mac,
            msi,
            PropertyIdentifier::ALARM_VALUES,
            None,
            vec![0x21, 2, 0x21, 3],
        )
        .await;
    assert_eq!(
        refusal(result),
        (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND, 2)
    );
    assert_eq!(read().await, alarm_values(&[1, 2]));

    // A refusal of the target names no element.
    let result = client
        .add_list_element(
            &server_mac,
            ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 99).unwrap(),
            PropertyIdentifier::ALARM_VALUES,
            None,
            vec![0x21, 2],
        )
        .await;
    assert_eq!(
        refusal(result),
        (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT, 0)
    );

    client
        .remove_list_element(
            &server_mac,
            msi,
            PropertyIdentifier::ALARM_VALUES,
            None,
            vec![0x21, 2],
        )
        .await
        .unwrap();
    assert_eq!(read().await, alarm_values(&[1]));

    server.stop().await.unwrap();
    client.stop().await.unwrap();
}
