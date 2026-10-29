//! Device administration preserves routed service fields and unconfirmed delivery.
use super::*;
use crate::discovery::{DiscoveredDevice, RoutedDeviceConfig};
use bacnet_encoding::apdu::{self, Apdu, ConfirmedRequest, ErrorPdu, RejectPdu, SimpleAck};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_services::device_mgmt::{
    DeviceCommunicationControlRequest, ReinitializeDeviceRequest, TimeSynchronizationRequest,
};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    EnableDisable, ErrorClass, ErrorCode, ObjectType, ReinitializedState, RejectReason,
    Segmentation,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

const INSTANCE: u32 = 1234;
const ROUTER: &[u8] = &[2];
const NETWORK: u16 = 100;
const STATION: &[u8] = &[5];

struct Peer {
    transport: LoopbackTransport,
    frames: mpsc::Receiver<ReceivedNpdu>,
    destinations: mpsc::UnboundedReceiver<MacAddr>,
    routed: bool,
    next_hop: &'static [u8],
}

async fn site(routed: bool) -> (BACnetClient<LoopbackTransport>, Peer) {
    let (mut local, mut remote) = LoopbackTransport::pair(&[1][..], ROUTER);
    local.set_broadcast_mac(&[0xff][..]);
    let destinations = local.record_unicast_destinations();
    let frames = remote.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(local)
        .apdu_timeout_ms(1000)
        .apdu_retries(2)
        .build()
        .await
        .unwrap();
    if routed {
        client
            .add_routed_device(RoutedDeviceConfig {
                instance: INSTANCE,
                router_mac: ROUTER.to_vec(),
                remote_network: NETWORK,
                remote_mac: STATION.to_vec(),
                max_apdu_length: 1476,
                segmentation_supported: Segmentation::NONE,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
    } else {
        client.add_device(INSTANCE, ROUTER).await.unwrap();
    }
    (
        client,
        Peer {
            transport: remote,
            frames,
            destinations,
            routed,
            next_hop: ROUTER,
        },
    )
}

impl Peer {
    async fn apdu(&mut self, expecting_reply: bool) -> Apdu {
        let frame = timeout(Duration::from_secs(5), self.frames.recv())
            .await
            .expect("request watchdog")
            .unwrap();
        assert!(!frame.link_layer_group);
        assert_eq!(
            self.destinations.recv().await.unwrap().as_slice(),
            self.next_hop
        );
        let npdu = decode_npdu(frame.npdu).unwrap();
        assert_eq!(
            npdu.expecting_reply, expecting_reply,
            "wrong expecting-reply flag"
        );
        assert_eq!(npdu.priority, NetworkPriority::Normal);
        if self.routed {
            let destination = npdu
                .destination
                .expect("administrative request lost DNET/DADR");
            assert_eq!(destination.network, NETWORK);
            assert_eq!(destination.mac_address.as_slice(), STATION);
        } else {
            assert!(npdu.destination.is_none());
        }
        apdu::decode_apdu(npdu.payload).unwrap()
    }
    async fn confirmed(&mut self, service: ConfirmedServiceChoice) -> ConfirmedRequest {
        let Apdu::ConfirmedRequest(request) = self.apdu(true).await else {
            panic!("expected confirmed request");
        };
        assert_eq!(request.service_choice, service);
        request
    }
    async fn send(&self, apdu: Apdu) {
        let mut payload = BytesMut::new();
        apdu::encode_apdu(&mut payload, &apdu).unwrap();
        let mut bytes = BytesMut::new();
        encode_npdu(
            &mut bytes,
            &Npdu {
                source: self.routed.then(|| NpduAddress {
                    network: NETWORK,
                    mac_address: MacAddr::from_slice(STATION),
                }),
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.transport.send_unicast(&bytes, &[1]).await.unwrap();
    }
    async fn ack(&self, request: &ConfirmedRequest) {
        self.send(Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        }))
        .await;
    }
    fn empty(&mut self) {
        assert!(self.frames.try_recv().is_err());
        assert!(self.destinations.try_recv().is_err());
    }
}

fn date() -> Date {
    Date {
        year: 124,
        month: 7,
        day: 4,
        day_of_week: 4,
    }
}
fn time() -> Time {
    Time {
        hour: 13,
        minute: 27,
        second: 39,
        hundredths: 51,
    }
}

#[tokio::test(start_paused = true)]
async fn device_mgmt_confirmed_fields_and_discovered_local_fallback() {
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        for (state, duration, password) in [
            (EnableDisable::ENABLE, None, None),
            (EnableDisable::DISABLE, Some(0), None),
            (
                EnableDisable::DISABLE_INITIATION,
                Some(23),
                Some("control-key".to_owned()),
            ),
        ] {
            let (result, ()) = tokio::join!(
                client.device_communication_control_from_device(
                    INSTANCE,
                    state,
                    duration,
                    password.clone()
                ),
                async {
                    let request = peer
                        .confirmed(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL)
                        .await;
                    assert_eq!(
                        DeviceCommunicationControlRequest::decode(&request.service_request)
                            .unwrap(),
                        DeviceCommunicationControlRequest {
                            time_duration: duration,
                            enable_disable: state,
                            password
                        }
                    );
                    peer.ack(&request).await;
                }
            );
            result.unwrap();
        }
        for (state, password) in [
            (ReinitializedState::WARMSTART, None),
            (
                ReinitializedState::START_BACKUP,
                Some("reinit-key".to_owned()),
            ),
        ] {
            let (result, ()) = tokio::join!(
                client.reinitialize_device_from_device(INSTANCE, state, password.clone()),
                async {
                    let request = peer
                        .confirmed(ConfirmedServiceChoice::REINITIALIZE_DEVICE)
                        .await;
                    assert_eq!(
                        ReinitializeDeviceRequest::decode(&request.service_request).unwrap(),
                        ReinitializeDeviceRequest {
                            reinitialized_state: state,
                            password
                        }
                    );
                    peer.ack(&request).await;
                }
            );
            result.unwrap();
        }
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn device_mgmt_confirmed_protocol_errors_and_rejects_are_preserved() {
    let (mut client, mut peer) = site(true).await;
    let (result, ()) = tokio::join!(
        client.device_communication_control_from_device(
            INSTANCE,
            EnableDisable::ENABLE,
            None,
            None
        ),
        async {
            let request = peer
                .confirmed(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL)
                .await;
            peer.send(Apdu::Error(ErrorPdu {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
                error_class: ErrorClass::SECURITY,
                error_code: ErrorCode::PASSWORD_FAILURE,
                error_data: Bytes::new(),
            }))
            .await;
        }
    );
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::SECURITY,
            code: ErrorCode::PASSWORD_FAILURE
        }))
    ));
    let (result, ()) = tokio::join!(
        client.reinitialize_device_from_device(INSTANCE, ReinitializedState::WARMSTART, None),
        async {
            let request = peer
                .confirmed(ConfirmedServiceChoice::REINITIALIZE_DEVICE)
                .await;
            peer.send(Apdu::Reject(RejectPdu {
                invoke_id: request.invoke_id,
                reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
            }))
            .await;
        }
    );
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Reject {
            reason: RejectReason::UNRECOGNIZED_SERVICE
        }))
    ));
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn device_mgmt_time_services_send_once_without_reply_and_localize_known_network() {
    for mode in 0..3 {
        let (mut client, mut peer) = site(mode != 0).await;
        if mode == 2 {
            client
                .network
                .local_network_number()
                .publish(bacnet_types::network_number::NetworkNumber::configured(NETWORK).unwrap());
            peer.routed = false;
            peer.next_hop = STATION;
        }
        for utc in [false, true] {
            // Send completes before any peer task runs or answers, even with retries configured.
            if utc {
                client
                    .utc_time_synchronization_from_device(INSTANCE, date(), time())
                    .await
                    .unwrap();
            } else {
                client
                    .time_synchronization_from_device(INSTANCE, date(), time())
                    .await
                    .unwrap();
            }
            let Apdu::UnconfirmedRequest(request) = peer.apdu(false).await else {
                panic!("expected unconfirmed time service");
            };
            assert_eq!(
                request.service_choice,
                if utc {
                    UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
                } else {
                    UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
                }
            );
            assert_eq!(
                TimeSynchronizationRequest::decode(&request.service_request).unwrap(),
                TimeSynchronizationRequest {
                    date: date(),
                    time: time()
                }
            );
            tokio::time::advance(Duration::from_secs(10)).await;
            peer.empty();
        }
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn device_mgmt_unknown_device_and_invalid_time_routes_send_nothing() {
    let (mut client, mut peer) = site(true).await;
    assert!(
        client
            .device_communication_control_from_device(
                INSTANCE + 1,
                EnableDisable::ENABLE,
                None,
                None
            )
            .await
            .is_err()
    );
    assert!(
        client
            .reinitialize_device_from_device(INSTANCE + 1, ReinitializedState::WARMSTART, None)
            .await
            .is_err()
    );
    assert!(
        client
            .time_synchronization_from_device(INSTANCE + 1, date(), time())
            .await
            .is_err()
    );
    assert!(
        client
            .utc_time_synchronization_from_device(INSTANCE + 1, date(), time())
            .await
            .is_err()
    );
    peer.empty();
    for (network, address) in [
        (0, vec![5]),
        (u16::MAX, vec![5]),
        (NETWORK, vec![]),
        (NETWORK, vec![5; 256]),
        (NETWORK, vec![0xff]),
    ] {
        if address == [0xff] {
            // A route naming this network must still name a single station.
            client
                .network
                .local_network_number()
                .publish(bacnet_types::network_number::NetworkNumber::configured(NETWORK).unwrap());
        }
        client.device_table.lock().await.upsert(DiscoveredDevice {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE + 1).unwrap(),
            mac_address: MacAddr::from_slice(ROUTER),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            max_segments_accepted: None,
            vendor_id: 0,
            last_seen: std::time::Instant::now(),
            source_network: Some(network),
            source_address: Some(MacAddr::from_slice(&address)),
        });
        assert!(
            client
                .time_synchronization_from_device(INSTANCE + 1, date(), time())
                .await
                .is_err()
        );
        assert!(
            client
                .utc_time_synchronization_from_device(INSTANCE + 1, date(), time())
                .await
                .is_err()
        );
        peer.empty();
    }
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn over_long_or_empty_passwords_fail_before_anything_is_sent() {
    // #2085: 21 characters (two octets each in UTF-8) never reach the wire;
    // #2094: nor does an empty password, which the caller omits instead.
    for (too_long, why, routed) in [
        ("é".repeat(21), "20-character limit", true),
        ("é".repeat(21), "20-character limit", false),
        (String::new(), "omit it", true),
        (String::new(), "omit it", false),
    ] {
        let (mut client, mut peer) = site(routed).await;
        let errors = [
            client
                .device_communication_control_from_device(
                    INSTANCE,
                    EnableDisable::DISABLE,
                    None,
                    Some(too_long.clone()),
                )
                .await
                .unwrap_err(),
            client
                .reinitialize_device_from_device(
                    INSTANCE,
                    ReinitializedState::COLDSTART,
                    Some(too_long.clone()),
                )
                .await
                .unwrap_err(),
            client
                .device_communication_control(
                    ROUTER,
                    EnableDisable::DISABLE,
                    None,
                    Some(too_long.clone()),
                )
                .await
                .unwrap_err(),
            client
                .reinitialize_device(
                    ROUTER,
                    ReinitializedState::COLDSTART,
                    Some(too_long.clone()),
                )
                .await
                .unwrap_err(),
        ];
        for err in errors {
            assert!(matches!(err, Error::Encode(_)), "{err:?}");
            assert!(err.to_string().contains(why), "{err}");
        }
        tokio::task::yield_now().await;
        peer.empty();
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}
