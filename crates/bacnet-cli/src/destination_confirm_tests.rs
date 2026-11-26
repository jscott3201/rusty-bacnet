//! An instance target is reached only where its device answered as it
//! (#2824): `bacnet write 1234 ...` and every other command by instance
//! first sends one Who-Is for 1234 (no row registered by hand claims it),
//! then asks each address the discovery table has claiming it which device
//! answers there, and the refusal names each address asked and what
//! answered. Station 2 is device 1234; station 3 answers as `three` says,
//! or not at all.

use bacnet_client::client::ClientConfig;
use bacnet_encoding::apdu::{Apdu, ComplexAck, UnconfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;
use tokio::sync::mpsc;

use super::*;

const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const TWO: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];
const THREE: [u8; 6] = [10, 0, 0, 3, 0xBA, 0xC0];
const INSTANCE: u32 = 1234;

/// The answer to `frame`, a ReadProperty of a Device object's
/// Object_Identifier, from the station it went to: station 2 names
/// 1234, station 3 names `three` (or says nothing for `None`).
fn answer(frame: &SentFrame, three: Option<u32>) -> Option<ReceivedNpdu> {
    if frame.broadcast {
        return None;
    }
    let named = match frame.mac.as_slice() {
        mac if mac == TWO => INSTANCE,
        mac if mac == THREE => three?,
        _ => return None,
    };
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(frame.decode_npdu().payload) else {
        return None;
    };
    let read = ReadPropertyRequest::decode(&request.service_request).ok()?;
    let mut value = BytesMut::new();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, named).unwrap();
    encode_property_value(&mut value, &PropertyValue::ObjectIdentifier(device)).unwrap();
    let mut service_ack = BytesMut::new();
    // The device's own identifier, as an answer by the wildcard names it.
    ReadPropertyACK {
        object_identifier: device,
        property_identifier: read.property_identifier,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut service_ack)
    .unwrap();
    let ack = Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: request.invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: service_ack.freeze(),
    });
    Some(received(&ack, &frame.mac, false))
}

/// `apdu` from `from`, as the client receives it.
fn received(apdu: &Apdu, from: &MacAddr, broadcast: bool) -> ReceivedNpdu {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let mut npdu = BytesMut::new();
    let frame = Npdu {
        payload: payload.freeze(),
        ..Npdu::default()
    };
    encode_npdu(&mut npdu, &frame).unwrap();
    ReceivedNpdu::unverified(npdu.freeze(), from.clone(), broadcast, Vec::new(), None)
}

/// An I-Am for 1234 from `from`, broadcast.
fn i_am(from: [u8; 6]) -> ReceivedNpdu {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 42,
    }
    .encode(&mut service);
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service.freeze(),
    });
    received(&apdu, &MacAddr::from_slice(&from), true)
}

/// A client that heard 1234 announced from each of `claims`, station 3
/// answering as `three`.
async fn site(claims: &[[u8; 6]], three: Option<u32>) -> (BACnetClient<TestTransport>, SendLog) {
    let (inbox, inbound) = mpsc::channel(32);
    for claim in claims {
        inbox.send(i_am(*claim)).await.unwrap();
    }
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(inbound)
        .on_send(move |frame| {
            let inbox = inbox.clone();
            async move {
                if let Some(reply) = answer(&frame, three) {
                    let _ = inbox.send(reply).await;
                }
                Ok(())
            }
        })
        .build();
    let sent = transport.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 500,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    for _ in 0..100 {
        if client.discovered_devices().await.len() == claims.len() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(client.discovered_devices().await.len(), claims.len());
    (client, sent)
}

/// The Who-Is frames sent.
fn who_is(sent: &SendLog) -> usize {
    sent.broadcasts()
        .iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS)
        })
        .count()
}

/// Requests sent to `to`.
fn asked(sent: &SendLog, to: [u8; 6]) -> usize {
    sent.unicasts()
        .iter()
        .filter(|frame| frame.mac.as_slice() == to)
        .count()
}

/// 1234 announced from station 3 alone, where nothing answers: the target
/// is refused after one question there, naming the address and saying it
/// didn't answer. The BVLL commands' lookup refuses it the same way.
#[tokio::test(start_paused = true)]
async fn a_forged_claim_is_asked_and_refused() {
    let (client, sent) = site(&[THREE], None).await;
    let wait = Duration::from_secs(1);
    let refused = resolve(&client, "1234", wait).await.unwrap_err();
    assert!(
        refused.contains("claimed at 10.0.0.3:47808") && refused.contains("it didn't answer"),
        "{refused}"
    );
    let refused = resolve_mac(&client, "1234", wait).await.unwrap_err();
    assert!(refused.contains("claimed at 10.0.0.3:47808"), "{refused}");
    assert_eq!(asked(&sent, THREE), 2, "one question a lookup");
    assert_eq!(asked(&sent, TWO), 0);
    assert_eq!(who_is(&sent), 2, "one Who-Is a lookup, first");
}

/// 1234 claimed at both stations: both are asked, and the one that answers
/// as 1234 is the destination; one answering as another device or not at
/// all is passed over. Both answering as 1234 is refused, naming both and
/// what each said.
#[tokio::test(start_paused = true)]
async fn a_shared_claim_goes_where_the_device_answers() {
    for three in [None, Some(INSTANCE + 1)] {
        let (client, sent) = site(&[TWO, THREE], three).await;
        let to = resolve(&client, "1234", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(to.address, DeviceAddress::local(&TWO).unwrap(), "{three:?}");
        assert_eq!(to.instance, Some(INSTANCE));
        assert_eq!(
            (asked(&sent, TWO), asked(&sent, THREE)),
            (1, 1),
            "{three:?}"
        );
    }
    let (client, _) = site(&[TWO, THREE], Some(INSTANCE)).await;
    let refused = resolve(&client, "1234", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        refused.contains("10.0.0.2:47808 (it answered as device 1234)")
            && refused.contains("10.0.0.3:47808 (it answered as device 1234)"),
        "{refused}"
    );
}

/// The device's own claim alone: one question, and the destination is its
/// address, with its instance.
#[tokio::test(start_paused = true)]
async fn the_device_named_by_its_instance_is_asked_once() {
    let (client, sent) = site(&[TWO], None).await;
    let to = resolve(&client, "1234", Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        to,
        Destination {
            address: DeviceAddress::local(&TWO).unwrap(),
            instance: Some(INSTANCE),
        }
    );
    assert_eq!(asked(&sent, TWO), 1);
}
