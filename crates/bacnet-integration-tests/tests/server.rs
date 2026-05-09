//! End-to-end integration tests: real BACnetClient ↔ real BACnetServer over loopback UDP.

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::time::Duration;

/// Build a server with a Device, an AnalogInput, and a BinaryValue.
async fn make_server() -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();

    // Device object (instance 1234)
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Integration Test Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();

    // AnalogInput (instance 1, present-value = 72.5)
    let mut ai = AnalogInputObject::new(1, "Zone Temp", 62).unwrap();
    ai.set_present_value(72.5);

    // BinaryValue (instance 1, default = inactive)
    let bv = BinaryValueObject::new(1, "Fan Status").unwrap();

    let ai_oid = ai.object_identifier();
    let bv_oid = bv.object_identifier();
    let dev_oid = device.object_identifier();

    // Update device object-list
    device.set_object_list(vec![dev_oid, ai_oid, bv_oid]);

    db.add(Box::new(device)).unwrap();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(bv)).unwrap();

    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0) // ephemeral
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await
        .unwrap()
}

async fn make_client() -> BACnetClient<BipTransport> {
    BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap()
}

#[path = "server/basic.rs"]
mod basic;
#[path = "server/dcc.rs"]
mod dcc;
#[path = "server/error_cov.rs"]
mod error_cov;
#[path = "server/routing_alarm.rs"]
mod routing_alarm;
#[path = "server/segmentation_rx.rs"]
mod segmentation_rx;
#[path = "server/segmentation_tx.rs"]
mod segmentation_tx;
