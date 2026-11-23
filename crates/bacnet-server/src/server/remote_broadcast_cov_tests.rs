//! A COV subscription comes from a request's SNET and SADR, which no router
//! writes as a broadcast, but a crafted or misrelayed request can: the
//! server takes it, and gives it unconfirmed notifications only (#2732).
//! Each confirmed report refused counts once; a confirmed subscriber at a
//! station on the same network is served. Ordinary and COV-multiple
//! subscriptions alike.

use super::*;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::PropertyReference;

fn av1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

/// A confirmed request relayed by [`ROUTER`] from `source` on [`NETWORK`].
fn routed_request(
    source: &[u8],
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    body: BytesMut,
) -> ReceivedNpdu {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: body.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            source: Some(NpduAddress {
                network: NETWORK,
                mac_address: MacAddr::from_slice(source),
            }),
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&ROUTER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// A SubscribeCOV for AV-1 from `source`, process `process`.
fn subscribe(source: &[u8], process: u32, confirmed: bool) -> ReceivedNpdu {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: av1(),
        issue_confirmed_notifications: Some(confirmed),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    let invoke_id = u8::try_from(process).unwrap();
    routed_request(
        source,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
        body,
    )
}

/// A SubscribeCOVPropertyMultiple for AV-1's Present_Value from `source`,
/// process `process`.
fn subscribe_multiple(source: &[u8], process: u32, confirmed: bool) -> ReceivedNpdu {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: process,
        issue_confirmed_notifications: confirmed,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: av1(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: Some(0.5),
                timestamped: false,
            }],
        }],
    }
    .encode(&mut body)
    .unwrap();
    let invoke_id = u8::try_from(process).unwrap();
    let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
    routed_request(source, invoke_id, service, body)
}

/// A started server with AV-1 on a B/IP-shaped link, the sender its
/// requests come in on, and its send log.
async fn cov_server() -> (
    BACnetServer<TestTransport>,
    mpsc::Sender<ReceivedNpdu>,
    SendLog,
) {
    let identity = bacnet_objects::DeviceIdentity::new(856, 0).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
            identity: identity.clone(),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        bacnet_objects::analog::AnalogValueObject::new(
            1,
            "AV-1",
            bacnet_types::enums::EngineeringUnits::DEGREES_CELSIUS,
        )
        .unwrap(),
    ))
    .unwrap();
    let (tx, rx) = mpsc::channel(8);
    let link = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC)
        .inbound(rx)
        .build();
    let log = link.sent();
    let config = ServerConfig {
        device: identity,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, link).await.unwrap();
    (server, tx, log)
}

/// The COV notifications on `log`, ordinary and Multiple, as (confirmed,
/// DADR), sorted.
fn cov_notifications(log: &SendLog) -> Vec<(bool, Vec<u8>)> {
    let mut sent: Vec<_> = log
        .unicasts()
        .into_iter()
        .filter_map(|frame| {
            let dadr = frame.decode_npdu().destination?.mac_address.to_vec();
            let confirmed = match frame.apdu() {
                Apdu::ConfirmedRequest(request)
                    if matches!(
                        request.service_choice,
                        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
                            | ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
                    ) =>
                {
                    true
                }
                Apdu::UnconfirmedRequest(request)
                    if matches!(
                        request.service_choice,
                        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
                            | UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
                    ) =>
                {
                    false
                }
                _ => return None,
            };
            Some((confirmed, dadr))
        })
        .collect();
    sent.sort();
    sent
}

/// The server's count of confirmed reports refused at a broadcast form.
async fn refused(server: &BACnetServer<TestTransport>) -> u64 {
    let counters = Arc::clone(server.cov_table.read().await.counters());
    counters
        .confirmed_broadcast_subscriber
        .load(Ordering::Relaxed)
}

async fn write_av1(server: &BACnetServer<TestTransport>, value: f32) {
    server
        .write_local(
            &av1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test(start_paused = true)]
async fn a_cov_subscriber_at_a_remote_broadcast_form_gets_no_confirmed_notification() {
    for form in FORMS {
        let (mut server, tx, log) = cov_server().await;
        for request in [
            subscribe(form, 1, true),
            subscribe(form, 2, false),
            subscribe(&STATION, 3, true),
        ] {
            tx.send(request).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        // Each subscription's first notification: none confirmed to `form`.
        assert_eq!(
            cov_notifications(&log),
            [(false, form.to_vec()), (true, STATION.to_vec())],
            "{form:02X?}"
        );
        assert_eq!(refused(&server).await, 1, "{form:02X?}");
        // A change is refused again, and counted again.
        log.take();
        write_av1(&server, 40.0).await;
        let sent = cov_notifications(&log);
        assert!(!sent.contains(&(true, form.to_vec())), "{sent:?}");
        assert!(sent.contains(&(false, form.to_vec())), "{sent:?}");
        assert_eq!(refused(&server).await, 2, "{form:02X?}");
        server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_cov_multiple_subscriber_at_a_remote_broadcast_form_gets_no_confirmed_notification() {
    for form in FORMS {
        let (mut server, tx, log) = cov_server().await;
        for request in [
            subscribe_multiple(form, 1, true),
            subscribe_multiple(form, 2, false),
            subscribe_multiple(&STATION, 3, true),
        ] {
            tx.send(request).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        write_av1(&server, 40.0).await;
        let sent = cov_notifications(&log);
        assert!(
            !sent.contains(&(true, form.to_vec())),
            "{form:02X?}: {sent:?}"
        );
        assert!(
            sent.contains(&(false, form.to_vec())),
            "{form:02X?}: {sent:?}"
        );
        assert!(
            sent.contains(&(true, STATION.to_vec())),
            "{form:02X?}: {sent:?}"
        );
        assert!(refused(&server).await >= 1, "{form:02X?}");
        server.stop().await.unwrap();
    }
}
