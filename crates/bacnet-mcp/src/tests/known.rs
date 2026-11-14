//! The server's known devices follow the client's table, one device per
//! address (#2560).

use std::time::Duration;

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::{ObjectType, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::{ALIASES, Lab, alias_mac};
use crate::Policy;

/// An I-Am for `instance`, as a device on the local network sends it.
fn i_am(instance: u32) -> BytesMut {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 260,
    }
    .encode(&mut service_request);
    let mut payload = BytesMut::new();
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    });
    encode_apdu(&mut payload, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    let message = Npdu {
        payload: payload.freeze(),
        ..Npdu::default()
    };
    encode_npdu(&mut npdu, &message).unwrap();
    npdu
}

/// Wait until `holds` is true of the lab, on the paused clock.
async fn until(lab: &Lab, holds: impl Fn(&Lab) -> bool, what: &str) {
    for _ in 0..1_000 {
        if holds(lab) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("never: {what}");
}

/// An address announcing another instance takes its old one out of the
/// devices the server knows: the client reports the old one lost.
#[tokio::test(start_paused = true)]
async fn an_address_announcing_another_instance_forgets_the_old_one() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let alias = ALIASES.start;
    let knows = |lab: &Lab, instance| lab.mcp.ctx().reach_known(instance);
    until(&lab, |lab| knows(lab, alias), "the alias is known").await;

    let transport = lab.client.transport();
    transport.inject(&i_am(7), &alias_mac(alias)).await;
    until(
        &lab,
        |lab| knows(lab, 7) && !knows(lab, alias),
        "7 replaces the alias",
    )
    .await;
    assert!(lab.client.get_device(alias).await.unwrap().is_none());
    lab.stop().await;
}
