//! A source Audit Device bound behind a router at a broadcast or group form
//! on its network (#2732): the router there would hand a confirmed record to
//! every station, so a confirmed Reporter has no route to it and reads
//! CONFIGURATION_ERROR, while an unconfirmed one still reaches it. Judged
//! from the shared test transport's send log; no router is learned, so each
//! routed record goes as this link's broadcast with its DNET and DADR. A
//! binding marked one station (#2738) lifts the forms that are a station on
//! some network.
use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::test_transport::SendLog;
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};

/// One MAC of each form `bacnet_npdu::RemoteBroadcast` names.
const FORMS: [&[u8]; 5] = [
    &[0xFF],
    &[0x00, 0x05],
    &[0xFF, 0x07],
    &[0xFF, 0xFF, 0xFF],
    &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
];

/// Addresses a binding marked one station reaches: the forms the mark
/// lifts (ARCNET's station 255, two-octet stations starting `0x00` or
/// `0xFF`, the ends of each range included), and the one octet `0x00`, which
/// the mark never refuses.
const MARKED_STATIONS: [&[u8]; 7] = [
    &[0x00],
    &[0xFF],
    &[0x00, 0x01],
    &[0x00, 0xFF],
    &[0xFF, 0x00],
    &[0xFF, 0xFE],
    &[0xFF, 0x07],
];

/// Forms still refused for a binding marked one station.
const MARKED_FORMS: [&[u8]; 4] = [
    &[0x00, 0x00],
    &[0xFF, 0xFF],
    &[0xFF, 0xFF, 0xFF],
    &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
];

/// A ClientOnly session at [`SESSION`] whose Reporter records DEVICE_RESET,
/// confirmed or not, to Device 999, bound at `mac` on network [`REMOTE`],
/// marked one station when `one_station`, with Device 998 bound at
/// [`STATION`] there. The inbound sender keeps the link open.
async fn session(
    confirmed: bool,
    mac: &[u8],
    one_station: bool,
) -> (
    EndpointSession<TestTransport>,
    SendLog,
    tokio::sync::mpsc::Sender<bacnet_transport::port::ReceivedNpdu>,
) {
    let (inbound, rx) = tokio::sync::mpsc::channel(4);
    let link = TestTransport::builder()
        .local_mac(&SESSION)
        .inbound(rx)
        .build();
    let log = link.sent();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::DEVICE_RESET);
    let mut db = provisioned();
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .unwrap()
        .configure(
            AuditLevel::AUDIT_ALL,
            flags,
            confirmed,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
    let session = EndpointSession::new(link, SessionRole::ClientOnly, SessionConfig::default())
        .unwrap()
        .with_database(db);
    let (device, mac) = (oid(ObjectType::DEVICE, 999), MacAddr::from_slice(mac));
    let session = if one_station {
        session.with_source_audit_routed_one_station_binding(device, REMOTE, mac)
    } else {
        session.with_source_audit_routed_device_binding(device, REMOTE, mac)
    };
    let mut session = session
        .with_source_audit_routed_device_binding(
            oid(ObjectType::DEVICE, 998),
            REMOTE,
            MacAddr::from_slice(&STATION),
        )
        .with_source_audit_reporter(selected());
    session.start().await.unwrap();
    (session, log, inbound)
}

/// The Audit records on `log`, taken, as (DNET, DADR, confirmed).
fn records(log: &SendLog) -> Vec<(u16, Vec<u8>, bool)> {
    log.take()
        .into_iter()
        .filter_map(|frame| {
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            let confirmed = match decode_apdu(npdu.payload).ok()? {
                Apdu::ConfirmedRequest(request)
                    if request.service_choice
                        == ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION =>
                {
                    true
                }
                Apdu::UnconfirmedRequest(request)
                    if request.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION =>
                {
                    false
                }
                _ => return None,
            };
            let to = npdu.destination.expect("a routed record");
            Some((to.network, to.mac_address.to_vec(), confirmed))
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_reporter_has_no_route_to_a_device_at_a_routed_broadcast_form() {
    for form in FORMS {
        for confirmed in [false, true] {
            let (mut session, log, _inbound) = session(confirmed, form, false).await;
            let case = format!("{form:02X?}, confirmed {confirmed}");
            let expected = if confirmed {
                Reliability::CONFIGURATION_ERROR
            } else {
                Reliability::NO_FAULT_DETECTED
            }
            .to_raw();
            assert_eq!(reliability(&session).await, expected, "{case}");
            session.report_device_reset_local().await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            let sent = records(&log);
            if confirmed {
                assert!(sent.is_empty(), "{case}: {sent:?}");
            } else {
                assert_eq!(sent, [(REMOTE, form.to_vec(), false)], "{case}");
            }
            assert_eq!(reliability(&session).await, expected, "{case}");
            session.stop().await.unwrap();
        }
    }
}

/// Clause 12.11.66: a confirmed Reporter's record of a recipient change
/// goes to both recipients or by global broadcast. A recipient at a
/// broadcast or group form has no route for it, so an unconfirmed global
/// broadcast stands in for its copy, on either side of the change.
#[tokio::test(start_paused = true)]
async fn a_recipient_change_broadcasts_for_a_side_at_a_routed_broadcast_form() {
    let (mut session, log, _inbound) = session(true, &[0xFF], false).await;
    for to in [998, 999] {
        let mut bytes = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(
            &mut bytes,
            &BACnetRecipient::Device(oid(ObjectType::DEVICE, to)),
        )
        .unwrap();
        session
            .database
            .as_ref()
            .unwrap()
            .write()
            .await
            .get_mut(&oid(ObjectType::DEVICE, 123))
            .unwrap()
            .write_property(
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(bytes.to_vec()),
                None,
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut sent = records(&log);
        sent.sort();
        assert_eq!(
            sent,
            [
                (REMOTE, STATION.to_vec(), true),
                (0xFFFF, Vec::new(), false)
            ],
            "to Device {to}"
        );
        // The confirmed copy's wait for its answer runs out first.
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    session.stop().await.unwrap();
}

/// A confirmed Reporter reaches a Device bound marked one station at each
/// form the mark lifts and at `0x00`, its record a confirmed request naming
/// the station;
/// at a form the mark doesn't lift it reads CONFIGURATION_ERROR and sends
/// nothing, as unmarked (#2738).
#[tokio::test(start_paused = true)]
async fn a_confirmed_reporter_reaches_a_device_bound_marked_one_station() {
    let lifted = MARKED_STATIONS.into_iter().map(|mac| (mac, true));
    for (mac, routed) in lifted.chain(MARKED_FORMS.into_iter().map(|mac| (mac, false))) {
        let (mut session, log, _inbound) = session(true, mac, true).await;
        let expected = if routed {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::CONFIGURATION_ERROR
        }
        .to_raw();
        assert_eq!(reliability(&session).await, expected, "{mac:02X?}");
        session.report_device_reset_local().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let sent = records(&log);
        if routed {
            assert_eq!(sent, [(REMOTE, mac.to_vec(), true)], "{mac:02X?}");
        } else {
            assert!(sent.is_empty(), "{mac:02X?}: {sent:?}");
        }
        session.stop().await.unwrap();
    }
}

/// A recipient change from a Device bound marked one station at `0xFF`
/// sends each side its confirmed copy, with no global broadcast standing in
/// for either.
#[tokio::test(start_paused = true)]
async fn a_recipient_change_reaches_a_side_bound_marked_one_station() {
    let (mut session, log, _inbound) = session(true, &[0xFF], true).await;
    for to in [998, 999] {
        let mut bytes = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(
            &mut bytes,
            &BACnetRecipient::Device(oid(ObjectType::DEVICE, to)),
        )
        .unwrap();
        session
            .database
            .as_ref()
            .unwrap()
            .write()
            .await
            .get_mut(&oid(ObjectType::DEVICE, 123))
            .unwrap()
            .write_property(
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(bytes.to_vec()),
                None,
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut sent = records(&log);
        sent.sort();
        assert_eq!(
            sent,
            [(REMOTE, STATION.to_vec(), true), (REMOTE, vec![0xFF], true),],
            "to Device {to}"
        );
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    session.stop().await.unwrap();
}
