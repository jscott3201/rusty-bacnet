//! Who-Is and Who-Has beside a second Device of a lower instance: the
//! server and the `Responder` both name the identity's Device in the I-Am
//! and the I-Have (#2186), never the database's lowest-instance one.
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::who_has::IHaveRequest;
use bacnet_services::who_is::IAmRequest;

/// Device 5, a Device the application adds after the server started (the
/// server refuses to start on a database whose lowest Device isn't the
/// identity's).
fn second_device() -> Box<DeviceObject> {
    let identity = bacnet_objects::DeviceIdentity::new(5, 0x105).unwrap();
    let config = DeviceConfig {
        identity,
        ..Default::default()
    };
    Box::new(DeviceObject::new(config).unwrap())
}

#[tokio::test(start_paused = true)]
async fn i_am_and_i_have_name_the_identity_device_beside_a_second_one() {
    let mut parity = Parity::start().await;
    let added = parity.server.database().write().await.add(second_device());
    added.unwrap();
    parity.db.add(second_device()).unwrap();
    assert_eq!(
        parity.db.selected_device(),
        Some(oid(ObjectType::DEVICE, 5))
    );
    let announced = |reply: Option<Apdu>| {
        let Some(Apdu::UnconfirmedRequest(pdu)) = reply else {
            panic!("expected an announcement, got {reply:?}");
        };
        if pdu.service_choice == UnconfirmedServiceChoice::I_AM {
            IAmRequest::decode(&pdu.service_request)
                .unwrap()
                .object_identifier
        } else {
            IHaveRequest::decode(&pdu.service_request)
                .unwrap()
                .device_identifier
        }
    };
    let ours = oid(ObjectType::DEVICE, DEVICE);
    let reply = parity.same("Who-Is, two Devices", who_is(None), 1476).await;
    assert_eq!(announced(reply), ours);
    let by_name = who_has(WhoHasObject::Name("BV-1".into()));
    let reply = parity.same("Who-Has, two Devices", by_name, 1476).await;
    assert_eq!(announced(reply), ours);
    parity.server.stop().await.unwrap();
}
