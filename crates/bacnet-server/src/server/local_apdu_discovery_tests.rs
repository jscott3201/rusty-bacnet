//! Direct handler controls await completion; no silence-as-completion assumption.
use super::*;

#[tokio::test]
async fn who_is_projects_frozen_identity_and_checks_membership() {
    let (transport, capture, _tx) = discovery_transport();
    let mut db = ObjectDatabase::new();
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 893).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(893, 0)
                .expect("valid Device identity")
                .with_max_apdu(1474)
                .expect("valid Device APDU"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut server = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(893, 0)
                .unwrap()
                .with_max_apdu(1474)
                .expect("valid Device APDU"),
            ..Default::default()
        },
        db,
        transport,
    )
    .await
    .unwrap();
    for (capacity, expected_sends) in [(Some(1474), 1), (Some(1476), 2), (None, 2), (Some(1474), 3)]
    {
        {
            let mut db = server.db.write().await;
            if db.get(&oid).is_some() {
                db.remove(&oid).unwrap();
            }
            if let Some(max_apdu_length) = capacity {
                db.add(Box::new(
                    DeviceObject::new(DeviceConfig {
                        identity: bacnet_objects::DeviceIdentity::new(893, 0)
                            .expect("valid Device identity")
                            .with_max_apdu(
                                u16::try_from(max_apdu_length).expect("Device APDU fits u16"),
                            )
                            .expect("valid Device APDU"),
                        ..Default::default()
                    })
                    .unwrap(),
                ))
                .unwrap();
            }
        }
        BACnetServer::handle_unconfirmed_request(
            &server.test_unconfirmed_services(),
            UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::WHO_IS,
                service_request: Bytes::new(),
            },
            &bacnet_network::layer::ReceivedApdu {
                router_response: None,
                direct_response: None,
                apdu: Bytes::new(),
                source_mac: MacAddr::from_slice(&[2]),
                ingress_network: None,
                source_network: None,
                link_layer_group: false,
                is_group: false,
                global_broadcast: false,
                data_attributes: vec![],
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            },
        )
        .await;
        assert_eq!(capture.unicasts().len(), expected_sends);
        assert_eq!(server.discovery_counters().i_am_sent, expected_sends as u64);
    }
    for frame in capture.unicasts() {
        let Apdu::UnconfirmedRequest(request) =
            decode_apdu(decode_npdu(frame.npdu).unwrap().payload).unwrap()
        else {
            panic!("I-Am expected")
        };
        assert_eq!(request.service_choice, UnconfirmedServiceChoice::I_AM);
        assert_eq!(
            IAmRequest::decode(&request.service_request)
                .unwrap()
                .max_apdu_length,
            1474
        );
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn actual_mstp_default_no_device_start_clamps_to_480_without_iam() {
    use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
    let (serial, _peer) = LoopbackSerial::pair();
    let mut server = BACnetServer::start(
        ServerConfig::default(),
        ObjectDatabase::new(),
        MstpTransport::new(serial, MstpConfig::default()),
    )
    .await
    .unwrap();
    assert_eq!(u32::from(server.config.device.max_apdu_length()), 480);
    assert!(server.broadcast_i_am().await.is_err());
    assert_eq!(server.discovery_counters().i_am_sent, 0);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn default_identity_device_starts_on_nonextended_mstp() {
    use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
    for (extended_frames, expected) in [(false, 480), (true, 1476)] {
        let (serial, _peer) = LoopbackSerial::pair();
        let mut server = BACnetServer::generic_builder()
            .device(bacnet_objects::DeviceIdentity::default())
            .transport(MstpTransport::new(
                serial,
                MstpConfig {
                    extended_frames,
                    ..Default::default()
                },
            ))
            .build()
            .await
            .expect("default identity resolves before stock generation");
        assert_eq!(server.config.device.max_apdu_length(), expected);
        assert_eq!(
            server
                .read_local(
                    &server.config.device.device_oid(),
                    PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
                    None
                )
                .await
                .unwrap(),
            PropertyValue::Unsigned(expected.into())
        );
        server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn selected_device_vendor_mismatch_fails_before_transport_start() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(1, 42).expect("valid Device identity"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let transport = super::super::test_transport::TestTransport::builder().build();
    let handle = transport.handle();
    let result = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(1, 555).expect("valid Device identity"),
            ..Default::default()
        },
        db,
        transport,
    )
    .await;
    let rejected = result.is_err();
    if let Ok(mut server) = result {
        server.stop().await.unwrap();
    }
    assert!(
        rejected,
        "mismatched vendor declarations must fail preflight"
    );
    assert_eq!(handle.starts(), 0);
}

#[tokio::test]
async fn custom_device_unknown_segmentation_fails_before_transport_start() {
    struct CustomDevice(DeviceObject);
    impl bacnet_objects::traits::BACnetObject for CustomDevice {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.0.object_identifier()
        }
        fn object_name(&self) -> &str {
            self.0.object_name()
        }
        fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
            self.0.property_list()
        }
        fn read_property(
            &self,
            property: PropertyIdentifier,
            index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            if property == PropertyIdentifier::SEGMENTATION_SUPPORTED {
                Ok(PropertyValue::Enumerated(64))
            } else {
                self.0.read_property(property, index)
            }
        }
        fn write_property(
            &mut self,
            property: PropertyIdentifier,
            index: Option<u32>,
            value: PropertyValue,
            priority: Option<u8>,
        ) -> Result<(), Error> {
            self.0.write_property(property, index, value, priority)
        }
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(CustomDevice(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    )))
    .unwrap();
    let transport = super::super::test_transport::TestTransport::never_start();
    let error = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("SEGMENTATION_SUPPORTED"),
        "{error}"
    );
}
