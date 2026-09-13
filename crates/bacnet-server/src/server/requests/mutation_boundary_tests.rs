use super::mutation_tests::{
    apdu, assert_denied, cases, oid, route, Fixture, TestTransport, SOURCE,
};
use super::*;
use crate::mutation::MutationTarget;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_services::file::{AtomicWriteFileRequest, FileWriteAccessMethod};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::enums::EnableDisable;
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn deny_all_leaves_read_discovery_and_password_authorized_dcc_working() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut fixture = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        false
    })));
    let mut bytes = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut bytes);
    let response = fixture
        .dispatch(ConfirmedServiceChoice::READ_PROPERTY, bytes.freeze(), 1)
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::ComplexAck(_)));

    fixture
        .db
        .write()
        .await
        .add(Box::new(
            DeviceObject::new(DeviceConfig::default()).unwrap(),
        ))
        .unwrap();
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &fixture.db,
        &fixture.network,
        &fixture.config,
        None,
        &fixture.state,
        &Arc::new(RwLock::new(DeviceBindingTable::new())),
        &Arc::new(DiscoveryLimiter::new(DiscoveryPolicy::default(), Some(1))),
        &Arc::new(TimeSyncLimiter::new(TimeSyncPolicy::default())),
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        },
        &ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: route(),
            link_layer_group: false,
            is_group: false,
            data_attributes: vec![],
            reply_tx: None,
        },
    )
    .await;
    let discovery = fixture
        .network
        .transport()
        .sent
        .lock()
        .unwrap()
        .pop()
        .unwrap();
    let npdu = decode_npdu(discovery).unwrap();
    assert_eq!(npdu.destination, route());
    let Apdu::UnconfirmedRequest(i_am) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected I-Am")
    };
    assert_eq!(i_am.service_choice, UnconfirmedServiceChoice::I_AM);
    assert_eq!(
        IAmRequest::decode(&i_am.service_request)
            .unwrap()
            .object_identifier,
        oid(ObjectType::DEVICE, 1)
    );

    fixture.config.dcc_policy = DccPolicy::RequirePassword;
    fixture.config.dcc_password = Some("local-policy-test".into());
    fixture.state.store(1, Ordering::Release);
    let mut bytes = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: Some("local-policy-test".into()),
    }
    .encode(&mut bytes)
    .unwrap();
    let response = fixture
        .dispatch(
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            bytes.freeze(),
            2,
        )
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    assert_eq!(fixture.state.load(Ordering::Acquire), 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn denied_subscription_cancellations_preserve_existing_entries() {
    for (service, bytes, target) in cases().into_iter().skip(7) {
        let mut fixture = Fixture::new(None);
        assert!(matches!(
            apdu(fixture.dispatch(service, bytes, 1).await.unwrap()),
            Apdu::SimpleAck(_)
        ));
        let before = fixture.table.read().await.len();
        assert!(before > 0);
        fixture.config.mutation_authorizer = Some(Arc::new(|_| false));
        let mut cancellation = BytesMut::new();
        match target {
            MutationTarget::SubscribeCov(mut request) => {
                request.issue_confirmed_notifications = None;
                request.lifetime = None;
                request.encode(&mut cancellation);
            }
            MutationTarget::SubscribeCovProperty(mut request) => {
                request.issue_confirmed_notifications = None;
                request.lifetime = None;
                request.encode(&mut cancellation);
            }
            MutationTarget::SubscribeCovPropertyMultiple(mut request) => {
                request.lifetime = None;
                request.max_notification_delay = None;
                request.encode(&mut cancellation);
            }
            _ => unreachable!(),
        }
        let cancellation = cancellation.freeze();
        assert_denied(
            fixture
                .dispatch(service, cancellation.clone(), 2)
                .await
                .unwrap(),
            service,
            2,
        );
        assert_eq!(fixture.table.read().await.len(), before);
        fixture.config.mutation_authorizer = None;
        assert!(matches!(
            apdu(fixture.dispatch(service, cancellation, 3).await.unwrap()),
            Apdu::SimpleAck(_)
        ));
        assert_eq!(fixture.table.read().await.len(), 0);
    }
}

#[tokio::test]
async fn allow_all_preserves_atomic_write_file_budget_abort() {
    let absent = Fixture::new(None);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let allow = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        true
    })));
    let mut bytes = BytesMut::new();
    AtomicWriteFileRequest {
        file_identifier: oid(ObjectType::FILE, 1),
        access: FileWriteAccessMethod::Stream {
            file_start_position: -1,
            file_data: vec![42; 16_385],
        },
    }
    .encode(&mut bytes);
    let bytes = bytes.freeze();
    let baseline = absent
        .dispatch(ConfirmedServiceChoice::ATOMIC_WRITE_FILE, bytes.clone(), 1)
        .await
        .unwrap();
    let response = allow
        .dispatch(ConfirmedServiceChoice::ATOMIC_WRITE_FILE, bytes, 1)
        .await
        .unwrap();
    assert_eq!(baseline, response);
    assert!(
        matches!(apdu(response), Apdu::Abort(a) if a.abort_reason == AbortReason::OUT_OF_RESOURCES)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        allow
            .read(oid(ObjectType::FILE, 1), PropertyIdentifier::FILE_SIZE)
            .await,
        PropertyValue::Unsigned(8)
    );
}
