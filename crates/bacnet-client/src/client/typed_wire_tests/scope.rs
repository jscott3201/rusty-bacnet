//! Who-Is, Who-Has and Who-Am-I to each scope and to one station: the local
//! broadcasts ops and the bindings encoded themselves, the network and
//! global ones the client sent, and the directed forms.

use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};
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
    let data = octets(|buf| WhoIsRequest { range: range() }.encode(buf));
    let choice = UnconfirmedServiceChoice::WHO_IS;
    pin!("who-is-bcast/local", |c| c
        .broadcast_unconfirmed(choice, &data));
    pin!("who-is-bcast/network", |c| c.who_is_network(DNET, range()));
    pin!("who-is-bcast/own", |c| async {
        c.network
            .local_network_number()
            .publish(NetworkNumber::configured(OWN).unwrap());
        c.who_is_network(OWN, range()).await
    });
    pin!("who-is-bcast/global", |c| c.who_is(range()));
}

#[tokio::test]
async fn who_has_to_each_scope() {
    let object = || WhoHasObject::Name("pump".to_string());
    let request = || WhoHasRequest {
        range: range(),
        object: object(),
    };
    let data = octets(|buf| request().encode(buf).unwrap());
    let choice = UnconfirmedServiceChoice::WHO_HAS;
    pin!("who-has-bcast/local", |c| c
        .broadcast_unconfirmed(choice, &data));
    pin!("who-has-bcast/network", |c| c
        .broadcast_network_unconfirmed(choice, &data, DNET));
    pin!("who-has-bcast/global", |c| c.who_has(object(), range()));
    pin!("who-has/local", |c| c
        .unconfirmed_request(PEER, choice, &data));
    pin!("who-has/routed", |c| c.unconfirmed_request(
        &routed(),
        choice,
        &data
    ));
}

#[tokio::test]
async fn who_am_i_to_each_scope() {
    let request = WhoAmIRequest {
        vendor_id: 555,
        model_name: "MX-1".to_string(),
        serial_number: "0042".to_string(),
    };
    let data = octets(|buf| request.encode(buf).unwrap());
    let choice = UnconfirmedServiceChoice::WHO_AM_I;
    pin!("who-am-i-bcast/local", |c| c
        .broadcast_unconfirmed(choice, &data));
    pin!("who-am-i-bcast/network", |c| c
        .broadcast_network_unconfirmed(choice, &data, DNET));
    pin!("who-am-i-bcast/global", |c| c
        .broadcast_global_unconfirmed(choice, &data));
    pin!("who-am-i/local", |c| c
        .unconfirmed_request(PEER, choice, &data));
    pin!("who-am-i/routed", |c| c.unconfirmed_request(
        &routed(),
        choice,
        &data
    ));
}
