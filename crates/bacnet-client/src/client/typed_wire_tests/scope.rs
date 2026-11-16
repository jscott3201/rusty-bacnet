//! Who-Is, Who-Has and Who-Am-I to each [`Broadcast`] scope and to one
//! station. The pins were recorded from the sends ops and the bindings
//! made from their own octets (the local broadcasts) and the client's
//! broadcast calls (the rest), before the scoped calls.

use bacnet_services::private_transfer::PrivateTransferRequest;
use bacnet_services::text_message::TextMessageRequest;
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_types::enums::MessagePriority;
use bacnet_types::network_number::NetworkNumber;

use super::*;

/// The client's own network, once it knows the number.
const OWN: u16 = 7;

fn range() -> Option<DeviceInstanceRange> {
    Some(DeviceInstanceRange::new(10, 20).unwrap())
}

fn routed() -> Station {
    Station::routed(ROUTER, DNET, DADR)
}

#[tokio::test]
async fn who_is_to_each_scope() {
    pin!("who-is-bcast/local", |c| c
        .who_is(Broadcast::Local, range()));
    pin!("who-is-bcast/network", |c| c
        .who_is(Broadcast::Network(DNET), range()));
    pin!("who-is-bcast/own", |c| async {
        c.network
            .local_network_number()
            .publish(NetworkNumber::configured(OWN).unwrap());
        c.who_is(Broadcast::Network(OWN), range()).await
    });
    pin!("who-is-bcast/global", |c| c
        .who_is(Broadcast::Global, range()));
}

#[tokio::test]
async fn who_has_to_each_scope() {
    let object = || WhoHasObject::Name("pump".to_string());
    pin!("who-has-bcast/local", |c| c.who_has(
        Broadcast::Local,
        object(),
        range()
    ));
    pin!("who-has-bcast/network", |c| c.who_has(
        Broadcast::Network(DNET),
        object(),
        range()
    ));
    pin!("who-has-bcast/global", |c| c.who_has(
        Broadcast::Global,
        object(),
        range()
    ));
    pin!("who-has/local", |c| c.who_has_directed(
        PEER,
        object(),
        range()
    ));
    pin!("who-has/routed", |c| c.who_has_directed(
        &routed(),
        object(),
        range()
    ));
}

#[tokio::test]
async fn who_am_i_to_each_scope() {
    let request = who_am_i_request();
    pin!("who-am-i-bcast/local", |c| c
        .who_am_i(Broadcast::Local, &request));
    pin!("who-am-i-bcast/network", |c| c
        .who_am_i(Broadcast::Network(DNET), &request));
    pin!("who-am-i-bcast/global", |c| c
        .who_am_i(Broadcast::Global, &request));
    pin!("who-am-i/local", |c| c.who_am_i_directed(PEER, &request));
    pin!("who-am-i/routed", |c| c
        .who_am_i_directed(&routed(), &request));
}

fn who_am_i_request() -> WhoAmIRequest {
    WhoAmIRequest {
        vendor_id: 555,
        model_name: "MX-1".to_string(),
        serial_number: "0042".to_string(),
    }
}

/// Network 0 and 65535 name no one remote network: each scoped call refuses
/// them before anything is sent, pointing at `Local` and `Global`.
#[tokio::test]
async fn a_network_broadcast_to_no_one_network_is_refused() {
    let wire = wire().await;
    let c = &wire.client;
    let request = who_am_i_request();
    for network in [0, u16::MAX] {
        let scope = Broadcast::Network(network);
        let pump = WhoHasObject::Name("pump".to_string());
        let results = [
            c.who_is(scope, None).await,
            c.who_has(scope, pump, None).await,
            c.who_am_i(scope, &request).await,
        ];
        for result in results {
            let Err(Error::Encoding(text)) = &result else {
                panic!("network {network}: {result:?}");
            };
            assert!(
                text.contains(&format!("not {network}"))
                    && text.contains("Broadcast::Local")
                    && text.contains("Broadcast::Global"),
                "{text}"
            );
        }
    }
    assert_eq!(wire.sent.len(), 0, "nothing sent");
}

/// A node that may not initiate, and then a closed seat, refuse every
/// scoped and directed unconfirmed call as they refuse the raw sends,
/// before anything is sent.
#[tokio::test]
async fn a_restricted_node_or_closed_seat_refuses_every_scope() {
    let wire = wire().await;
    let c = &wire.client;
    let request = who_am_i_request();
    let transfer = PrivateTransferRequest {
        vendor_id: 555,
        service_number: 1,
        service_parameters: None,
    };
    let message = TextMessageRequest {
        source_device: oid(ObjectType::DEVICE, 9),
        message_class: None,
        message_priority: MessagePriority::NORMAL,
        message: "hello".to_string(),
    };
    let pump = || WhoHasObject::Name("pump".to_string());
    for sealed in [false, true] {
        if sealed {
            c.device_table.asking.restrict(false);
            c.device_table.asking.seal();
        } else {
            c.device_table.asking.restrict(true);
        }
        let mut results = Vec::new();
        for scope in [
            Broadcast::Local,
            Broadcast::Network(DNET),
            Broadcast::Global,
        ] {
            results.push(c.who_is(scope, range()).await);
            results.push(c.who_has(scope, pump(), range()).await);
            results.push(c.who_am_i(scope, &request).await);
        }
        results.push(c.who_has_directed(PEER, pump(), range()).await);
        results.push(c.who_am_i_directed(PEER, &request).await);
        results.push(c.unconfirmed_private_transfer(PEER, &transfer).await);
        results.push(c.unconfirmed_text_message(PEER, &message).await);
        for result in results {
            if sealed {
                assert!(
                    matches!(&result, Err(Error::Encoding(text)) if text.contains("seat has closed")),
                    "{result:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(Error::InitiationDisabled { sent: false })),
                    "{result:?}"
                );
            }
        }
    }
    assert_eq!(wire.sent.len(), 0, "nothing sent");
}
