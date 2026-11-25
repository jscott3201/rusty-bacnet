//! The rarer services' arms no other test reaches through the wire (#2558).
use super::*;
use crate::server::cov_wire_test_support::Harness;
use crate::server::truncated_request_wire_tests::answer_to;
use bacnet_services::text_message::TextMessageRequest;
use bacnet_types::enums::MessagePriority;

/// A well-formed ConfirmedTextMessage is acknowledged with a SimpleAck that
/// names the service; the malformed ones' Reject is covered with the other
/// truncated requests.
#[tokio::test(start_paused = true)]
async fn a_well_formed_text_message_draws_a_simple_ack() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let mut body = BytesMut::new();
    TextMessageRequest {
        source_device: ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
        message_class: None,
        message_priority: MessagePriority::NORMAL,
        message: "boiler 2 on manual".into(),
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE;
    match answer_to(&mut h, service, &body).await {
        Apdu::SimpleAck(ack) => {
            assert_eq!(ack.invoke_id, h.invoke_id);
            assert_eq!(ack.service_choice, service);
        }
        other => panic!("a ConfirmedTextMessage drew {other:?}"),
    }
    h.server.stop().await.unwrap();
}
