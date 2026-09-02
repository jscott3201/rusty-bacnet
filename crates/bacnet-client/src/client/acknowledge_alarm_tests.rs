use super::*;

use bacnet_encoding::apdu::{decode_apdu, SimpleAck};
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{EventState, ObjectType};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};

fn time(hour: u8) -> BACnetTimeStamp {
    BACnetTimeStamp::Time(Time {
        hour,
        minute: 2,
        second: 3,
        hundredths: 4,
    })
}

fn date_time(day: u8) -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: 126,
            month: 9,
            day,
            day_of_week: 3,
        },
        time: Time {
            hour: 5,
            minute: 6,
            second: 7,
            hundredths: 8,
        },
    }
}

#[tokio::test]
async fn canonical_acknowledge_alarm_preserves_every_caller_supplied_field() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let requests = vec![
        AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 11,
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state_acknowledged: EventState::HIGH_LIMIT.to_raw(),
            timestamp: BACnetTimeStamp::SequenceNumber(101),
            acknowledgment_source: "sequence-to-time".into(),
            time_of_acknowledgment: time(9),
        },
        AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 22,
            event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
            event_state_acknowledged: EventState::FAULT.to_raw(),
            timestamp: time(10),
            acknowledgment_source: "time-to-date-time".into(),
            time_of_acknowledgment: date_time(2),
        },
        AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 33,
            event_object_identifier: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 3)
                .unwrap(),
            event_state_acknowledged: EventState::NORMAL.to_raw(),
            timestamp: date_time(3),
            acknowledgment_source: "date-time-to-sequence".into(),
            time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(303),
        },
    ];
    let expected = requests.clone();

    let responder = tokio::spawn(async move {
        for expected_request in expected {
            let received = timeout(Duration::from_secs(1), remote_rx.recv())
                .await
                .expect("remote timed out")
                .expect("remote channel closed");
            let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu).unwrap() else {
                panic!("expected ConfirmedRequest");
            };
            assert_eq!(
                request.service_choice,
                ConfirmedServiceChoice::ACKNOWLEDGE_ALARM
            );
            assert_eq!(
                AcknowledgeAlarmRequest::decode(&request.service_request).unwrap(),
                expected_request
            );

            let mut ack = BytesMut::new();
            encode_apdu(
                &mut ack,
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                }),
            )
            .unwrap();
            remote_network
                .send_apdu(&ack, &received.source_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();
        }
        remote_network.stop().await.unwrap();
    });

    for request in &requests {
        client
            .acknowledge_alarm_request(&remote_mac, request)
            .await
            .unwrap();
    }

    responder.await.unwrap();
    client.stop().await.unwrap();
}
