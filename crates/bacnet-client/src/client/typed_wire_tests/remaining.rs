//! The services only the bindings sent, from octets they encoded and
//! answers they decoded themselves: PrivateTransfer, TextMessage,
//! LifeSafetyOperation, the VT services and SubscribeCOVPropertyMultiple.

use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::private_transfer::{PrivateTransferAck, PrivateTransferRequest};
use bacnet_services::text_message::{MessageClass, TextMessageRequest};
use bacnet_services::virtual_terminal::{
    VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{LifeSafetyOperation, MessagePriority, VTClass};

use super::*;

fn routed() -> Station {
    Station::routed(ROUTER, DNET, DADR)
}

fn private_transfer() -> PrivateTransferRequest {
    PrivateTransferRequest {
        vendor_id: 555,
        service_number: 7,
        service_parameters: Some(vec![0x21, 0x05]),
    }
}

#[tokio::test]
async fn confirmed_private_transfer() {
    let ack = Answer::Complex(octets(|buf| {
        PrivateTransferAck {
            vendor_id: 555,
            service_number: 7,
            result_block: Some(vec![0x21, 0x06]),
        }
        .encode(buf)
    }));
    let choice = ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER;
    let data = octets(|buf| private_transfer().encode(buf));
    pin!("cpt/local", ack.clone(), |c| async {
        let answer = c.confirmed_request(PEER, choice, &data).await?;
        PrivateTransferAck::decode(&answer)
    });
    pin!("cpt/routed", ack, |c| async {
        let answer = c.confirmed_request(&routed(), choice, &data).await?;
        PrivateTransferAck::decode(&answer)
    });
    pin!("cpt-error/local", Answer::Error, |c| async {
        let answer = c.confirmed_request(PEER, choice, &data).await?;
        PrivateTransferAck::decode(&answer)
    });
}

#[tokio::test]
async fn unconfirmed_private_transfer() {
    let choice = UnconfirmedServiceChoice::UNCONFIRMED_PRIVATE_TRANSFER;
    let data = octets(|buf| private_transfer().encode(buf));
    pin!("upt/local", |c| c.unconfirmed_request(PEER, choice, &data));
    pin!("upt/routed", |c| c.unconfirmed_request(
        &routed(),
        choice,
        &data
    ));
}

fn text_message(class: Option<MessageClass>) -> TextMessageRequest {
    TextMessageRequest {
        source_device: oid(ObjectType::DEVICE, 9),
        message_class: class,
        message_priority: MessagePriority::URGENT,
        message: "check pump".to_string(),
    }
}

#[tokio::test]
async fn text_messages() {
    let text = || text_message(Some(MessageClass::Text("maint".to_string())));
    let numeric = || text_message(Some(MessageClass::Numeric(3)));
    let confirmed = ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE;
    let unconfirmed = UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE;
    let encoded = |request: TextMessageRequest| octets(|buf| request.encode(buf).unwrap());
    let (text_data, numeric_data) = (encoded(text()), encoded(numeric()));
    pin!("ctm/local", Answer::Simple, |c| async {
        c.confirmed_request(PEER, confirmed, &text_data)
            .await
            .map(drop)
    });
    pin!("ctm/routed", Answer::Simple, |c| async {
        c.confirmed_request(&routed(), confirmed, &text_data)
            .await
            .map(drop)
    });
    pin!("utm/local", |c| c.unconfirmed_request(
        PEER,
        unconfirmed,
        &numeric_data
    ));
    pin!("utm/routed", |c| c.unconfirmed_request(
        &routed(),
        unconfirmed,
        &numeric_data
    ));
}

#[tokio::test]
async fn life_safety_operation() {
    let request = LifeSafetyOperationRequest {
        requesting_process_identifier: 4,
        requesting_source: "operator".to_string(),
        request: LifeSafetyOperation::SILENCE_AUDIBLE,
        object_identifier: Some(oid(ObjectType::LIFE_SAFETY_POINT, 2)),
    };
    let choice = ConfirmedServiceChoice::LIFE_SAFETY_OPERATION;
    let data = octets(|buf| request.encode(buf).unwrap());
    pin!("lso/local", Answer::Simple, |c| async {
        c.confirmed_request(PEER, choice, &data).await.map(drop)
    });
    pin!("lso/routed", Answer::Simple, |c| async {
        c.confirmed_request(&routed(), choice, &data)
            .await
            .map(drop)
    });
}

#[tokio::test]
async fn virtual_terminal() {
    let open = VTOpenRequest {
        vt_class: VTClass::DEC_VT100,
        local_vt_session_identifier: 5,
    };
    let opened = Answer::Complex(octets(|buf| {
        VTOpenAck {
            remote_vt_session_identifier: 29,
        }
        .encode(buf)
    }));
    let open_data = octets(|buf| open.encode(buf));
    let choice = ConfirmedServiceChoice::VT_OPEN;
    pin!("vt-open/local", opened.clone(), |c| async {
        VTOpenAck::decode(&c.confirmed_request(PEER, choice, &open_data).await?)
    });
    pin!("vt-open/routed", opened, |c| async {
        VTOpenAck::decode(&c.confirmed_request(&routed(), choice, &open_data).await?)
    });

    let close = VTCloseRequest {
        list_of_remote_vt_session_identifiers: vec![29, 30],
    };
    let close_data = octets(|buf| close.encode(buf).unwrap());
    let choice = ConfirmedServiceChoice::VT_CLOSE;
    pin!("vt-close/local", Answer::Simple, |c| async {
        c.confirmed_request(PEER, choice, &close_data)
            .await
            .map(drop)
    });
    pin!("vt-close/routed", Answer::Simple, |c| async {
        c.confirmed_request(&routed(), choice, &close_data)
            .await
            .map(drop)
    });

    let data = VTDataRequest {
        vt_session_identifier: 29,
        vt_new_data: b"ls\r".to_vec(),
        vt_data_flag: true,
    };
    let partial = Answer::Complex(octets(|buf| {
        VTDataAck::Partial {
            accepted_octet_count: 2,
        }
        .encode(buf)
    }));
    let data_data = octets(|buf| data.encode(buf));
    let choice = ConfirmedServiceChoice::VT_DATA;
    pin!("vt-data/local", partial.clone(), |c| async {
        VTDataAck::decode(&c.confirmed_request(PEER, choice, &data_data).await?)
    });
    pin!("vt-data/routed", partial, |c| async {
        VTDataAck::decode(&c.confirmed_request(&routed(), choice, &data_data).await?)
    });
}

#[tokio::test]
async fn subscribe_cov_property_multiple() {
    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 12,
        issue_confirmed_notifications: false,
        lifetime: Some(600),
        max_notification_delay: Some(5),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PV,
                    property_array_index: None,
                },
                cov_increment: Some(0.5),
                timestamped: true,
            }],
        }],
    };
    let choice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
    let data = octets(|buf| request.encode(buf).unwrap());
    pin!("scpm/local", Answer::Simple, |c| async {
        c.confirmed_request(PEER, choice, &data).await.map(drop)
    });
    pin!("scpm/routed", Answer::Simple, |c| async {
        c.confirmed_request(&routed(), choice, &data)
            .await
            .map(drop)
    });
}
