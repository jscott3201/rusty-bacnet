//! Consumer-level proof: ClientOnly recipient writes need no private DB access.
use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_endpoint::{bip::BipEndpointBuilder, session::SessionRole, DeviceIdentity};
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_objects::audit::AuditReporterObject;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_transport::{bip::BipTransport, bvll::decode_bip_mac};
use bacnet_types::{
    constructed::{BACnetAuditNotification, BACnetRecipient},
    enums::{AuditLevel, AuditOperation, ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::{
    sync::mpsc,
    time::{timeout, Duration},
};

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}
fn device(instance: u32) -> BACnetRecipient {
    BACnetRecipient::Device(oid(ObjectType::DEVICE, instance))
}
async fn logger() -> (
    NetworkLayer<BipTransport>,
    mpsc::Receiver<ReceivedApdu>,
    SocketAddrV4,
) {
    let mut net = NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    ));
    let rx = net.start().await.unwrap();
    let (ip, port) = decode_bip_mac(net.local_mac()).unwrap();
    (net, rx, SocketAddrV4::new(ip.into(), port))
}
async fn record(rx: &mut mpsc::Receiver<ReceivedApdu>) -> BACnetAuditNotification {
    let packet = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Apdu::UnconfirmedRequest(pdu) = decode_apdu(packet.apdu).unwrap() else {
        panic!("unconfirmed Audit record")
    };
    let mut request = AuditNotificationRequest::decode(&pdu.service_request).unwrap();
    assert_eq!(request.notifications.len(), 1);
    request.notifications.remove(0)
}
fn encoded(recipient: &BACnetRecipient) -> Vec<u8> {
    let mut bytes = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut bytes, recipient);
    bytes.to_vec()
}

#[tokio::test]
async fn client_only_public_recipient_write_delivers_old_and_new_and_preserves_noops() {
    let (mut old, mut old_rx, old_addr) = logger().await;
    let (mut new, mut new_rx, new_addr) = logger().await;
    let mut db = DeviceIdentity::new(123, 42)
        .unwrap()
        .build_database()
        .unwrap();
    db.get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(device(999))
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "Source").unwrap();
    reporter.set_audit_level(AuditLevel::NONE).unwrap();
    reporter.set_issue_confirmed_notifications(false).unwrap();
    db.add(Box::new(reporter)).unwrap();
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ClientOnly)
        .database(db)
        .source_audit_device_binding(oid(ObjectType::DEVICE, 999), old_addr)
        .source_audit_device_binding(oid(ObjectType::DEVICE, 1000), new_addr)
        .build_session()
        .unwrap()
        .with_source_audit_reporter(oid(ObjectType::AUDIT_REPORTER, 1));
    assert!(session
        .write_audit_recipient(Some(device(1000)))
        .await
        .is_err());
    session.start().await.unwrap();
    assert!(old_rx.try_recv().is_err() && new_rx.try_recv().is_err());
    session
        .write_audit_recipient(Some(device(1000)))
        .await
        .unwrap();
    let first = record(&mut old_rx).await;
    assert_eq!(first, record(&mut new_rx).await);
    assert_eq!(first.operation, AuditOperation::WRITE);
    assert_eq!(first.source_device, device(123));
    assert_eq!(first.target_device, device(123));
    assert_eq!(first.target_object, Some(oid(ObjectType::DEVICE, 123)));
    assert_eq!(
        first.target_property.unwrap().property_identifier,
        PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT
    );
    assert_eq!(first.invoke_id, None);
    assert_eq!(first.current_value, Some(encoded(&device(999))));
    assert_eq!(first.target_value, Some(encoded(&device(1000))));
    session.write_audit_recipient(None).await.unwrap();
    session
        .write_audit_recipient(Some(device(1000)))
        .await
        .unwrap();
    assert!(session
        .write_audit_recipient(Some(device(1001)))
        .await
        .is_err());
    assert!(session
        .write_audit_recipient(Some(BACnetRecipient::Device(oid(
            ObjectType::ANALOG_VALUE,
            1
        ))))
        .await
        .is_err());
    session
        .write_audit_recipient(Some(device(999)))
        .await
        .unwrap();
    let second = record(&mut old_rx).await;
    assert_eq!(second, record(&mut new_rx).await);
    assert_eq!(second.current_value, Some(encoded(&device(1000))));
    assert_eq!(second.target_value, Some(encoded(&device(999))));
    session.stop().await.unwrap();
    assert!(session.write_audit_recipient(None).await.is_err());
    assert!(old_rx.try_recv().is_err() && new_rx.try_recv().is_err());
    old.stop().await.unwrap();
    new.stop().await.unwrap();
}
