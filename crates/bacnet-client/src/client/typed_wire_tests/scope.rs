//! Who-Is, Who-Has and Who-Am-I to each [`Broadcast`] scope and to one
//! station. The pins were recorded from the sends ops and the bindings
//! made from their own octets (the local broadcasts) and the client's
//! broadcast calls (the rest), before the scoped calls.

use bacnet_services::private_transfer::PrivateTransferRequest;
use bacnet_services::text_message::TextMessageRequest;
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject};
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

/// An I-Have from `device` for AI 1 named "pump": from `PEER` on this link,
/// or relayed by `ROUTER` from `routed`'s network and MAC.
async fn i_have(wire: &Wire, device: u32, routed: Option<(u16, &[u8])>) {
    let mut body = BytesMut::new();
    IHaveRequest {
        device_identifier: oid(ObjectType::DEVICE, device),
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        object_name: "pump".to_string(),
    }
    .encode(&mut body)
    .unwrap();
    let apdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_HAVE,
        service_request: body.freeze(),
    });
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &apdu).unwrap();
    let npdu = Npdu {
        source: routed.map(|(network, mac)| NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let from = if routed.is_some() { ROUTER } else { PEER };
    let received = ReceivedNpdu::unverified(
        buf.freeze(),
        MacAddr::from_slice(from),
        false,
        Vec::new(),
        None,
    );
    wire.inbox.send(received).await.unwrap();
}

/// A Who-Has's answers are those from the scope asked
/// ([`Broadcast::answered_from`]): device 1 answers from this link, device
/// 2 through the router from network 2, and device 3 through it from
/// network `OWN`, which is this network once the client knows the number.
#[tokio::test]
async fn who_has_replies_keep_the_answers_from_the_scope_asked() {
    let cases: [(Broadcast, Option<u16>, &[u32]); 6] = [
        (Broadcast::Local, None, &[1]),
        (Broadcast::Local, Some(OWN), &[1, 3]),
        (Broadcast::Network(DNET), None, &[2]),
        (Broadcast::Network(OWN), Some(OWN), &[1, 3]),
        (Broadcast::Global, None, &[1, 2, 3]),
        (Broadcast::Global, Some(OWN), &[1, 2, 3]),
    ];
    for (scope, own, expected) in cases {
        let wire = wire().await;
        let c = &wire.client;
        if let Some(own) = own {
            c.network
                .local_network_number()
                .publish(NetworkNumber::configured(own).unwrap());
        }
        let object = WhoHasObject::Name("pump".to_string());
        let wait = Duration::from_millis(300);
        let answer = async {
            tokio::time::timeout(Duration::from_secs(5), wire.sent.wait_for_len(1))
                .await
                .expect("the Who-Has goes out");
            i_have(&wire, 1, None).await;
            i_have(&wire, 2, Some((DNET, DADR))).await;
            i_have(&wire, 3, Some((OWN, &[0x33]))).await;
        };
        let (replies, ()) = tokio::join!(c.who_has_replies(scope, object, None, wait), answer);
        let mut kept: Vec<u32> = replies
            .unwrap()
            .iter()
            .map(|reply| reply.i_have.device_identifier.instance_number())
            .collect();
        kept.sort_unstable();
        assert_eq!(kept, expected, "{scope:?}, own network {own:?}");
    }
}

/// The forms the other pins leave out: a Who-Is with no range, a Who-Has
/// by identifier, and a Who-Has and a Who-Am-I to the client's own
/// network, which go as local broadcasts.
#[tokio::test]
async fn forms_without_a_range_or_to_the_own_network() {
    let own = || NetworkNumber::configured(OWN).unwrap();
    pin!("who-is-bcast/global-all", |c| c
        .who_is(Broadcast::Global, None));
    let by_id = || WhoHasObject::Identifier(oid(ObjectType::ANALOG_INPUT, 1));
    pin!("who-has-bcast/global-id", |c| c.who_has(
        Broadcast::Global,
        by_id(),
        None
    ));
    let pump = || WhoHasObject::Name("pump".to_string());
    pin!("who-has-bcast/own", |c| async {
        c.network.local_network_number().publish(own());
        c.who_has(Broadcast::Network(OWN), pump(), range()).await
    });
    let request = who_am_i_request();
    pin!("who-am-i-bcast/own", |c| async {
        c.network.local_network_number().publish(own());
        c.who_am_i(Broadcast::Network(OWN), &request).await
    });
}
