//! Endpoints whose objects come from point lists (#2445).
//!
//! Demonstrates:
//! - Building an endpoint's object database from a `PointList`
//! - Two endpoints on 127.0.0.1, each one device owning one B/IP socket
//!   that serves its own points and, through its client role, reads the
//!   other's
//!
//! Run from the workspace root with:
//! `cargo run -p bacnet-integration-tests --example point_list_endpoint`

#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use bacnet_encoding::primitives::decode_application_value;
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::{DeviceIdentity, EndpointSession, SessionRole};
use bacnet_objects::points::{
    Access, Analog, Binary, DeviceInfo, Point, PointList, build_database,
};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::net::Ipv4Addr;

/// An endpoint serving `points` as device `instance`, started.
async fn start(
    instance: u32,
    name: &str,
    points: Vec<Point>,
) -> Result<EndpointSession<BipTransport>, Box<dyn std::error::Error>> {
    // Vendor 0 is a placeholder: use the identifier ASHRAE assigned you.
    let identity = DeviceIdentity::new(instance, 0)?.with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
    let list = PointList::new(DeviceInfo::new(instance).with_name(name), points);
    let database = build_database(list, &identity)?;
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST)
        .role(SessionRole::Both)
        .database(database)
        .device(identity)
        .build_session()?;
    session.start().await?;
    Ok(session)
}

/// `session`'s B/IP address as a MAC address.
fn mac_of(session: &EndpointSession<BipTransport>) -> Result<[u8; 6], &'static str> {
    let address = session.bip_local_address().ok_or("not started")?;
    Ok(encode_bip_mac(address.ip().octets(), address.port()))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let pump = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1)?;
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    let mut plant = start(
        2001,
        "chiller-plant",
        vec![
            Point::new(
                temp,
                "plant.chw-supply-temp",
                Analog::new(celsius, Access::ReadOnly),
            )
            .with_description("Chilled water supply temperature"),
        ],
    )
    .await?;
    let mut ahu = start(
        2002,
        "ahu-2",
        vec![Point::new(
            pump,
            "ahu-2.coil-pump",
            Binary::new(Access::Commandable {
                relinquish_default: Some(BinaryPV::INACTIVE),
            }),
        )],
    )
    .await?;

    // Each endpoint reads the other's point by name, through the socket it
    // also serves on.
    for (reader, peer, object) in [(&ahu, &plant, temp), (&plant, &ahu, pump)] {
        let client = reader.client().ok_or("a client role")?;
        let ack = client
            .read_property(
                &mac_of(peer)?,
                object,
                PropertyIdentifier::OBJECT_NAME,
                None,
            )
            .await?;
        let (name, _) = decode_application_value(&ack.property_value, 0)?;
        println!("{object}: {name:?}");
    }

    ahu.stop().await?;
    plant.stop().await?;
    println!("Done.");
    Ok(())
}
