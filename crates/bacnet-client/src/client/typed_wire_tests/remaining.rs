//! The services only the bindings sent, from octets they encoded and
//! answers they decoded themselves, until they had typed calls:
//! PrivateTransfer, TextMessage, LifeSafetyOperation, the VT services and
//! SubscribeCOVPropertyMultiple. The pins were recorded from those sends.

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
    let request = private_transfer();
    pin!("cpt/local", ack.clone(), |c| c
        .confirmed_private_transfer(PEER, &request));
    pin!("cpt/routed", ack, |c| c
        .confirmed_private_transfer(&routed(), &request));
    pin!("cpt-error/local", Answer::Error, |c| c
        .confirmed_private_transfer(PEER, &request));
}

#[tokio::test]
async fn unconfirmed_private_transfer() {
    let request = private_transfer();
    pin!("upt/local", |c| c
        .unconfirmed_private_transfer(PEER, &request));
    pin!("upt/routed", |c| c
        .unconfirmed_private_transfer(&routed(), &request));
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
    let text = text_message(Some(MessageClass::Text("maint".to_string())));
    let numeric = text_message(Some(MessageClass::Numeric(3)));
    pin!("ctm/local", Answer::Simple, |c| c
        .confirmed_text_message(PEER, &text));
    pin!("ctm/routed", Answer::Simple, |c| c
        .confirmed_text_message(&routed(), &text));
    pin!("utm/local", |c| c.unconfirmed_text_message(PEER, &numeric));
    pin!("utm/routed", |c| c
        .unconfirmed_text_message(&routed(), &numeric));
}

#[tokio::test]
async fn life_safety_operation() {
    let request = LifeSafetyOperationRequest {
        requesting_process_identifier: 4,
        requesting_source: "operator".to_string(),
        request: LifeSafetyOperation::SILENCE_AUDIBLE,
        object_identifier: Some(oid(ObjectType::LIFE_SAFETY_POINT, 2)),
    };
    pin!("lso/local", Answer::Simple, |c| c
        .life_safety_operation(PEER, &request));
    pin!("lso/routed", Answer::Simple, |c| c
        .life_safety_operation(&routed(), &request));
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
    pin!("vt-open/local", opened.clone(), |c| c.vt_open(PEER, &open));
    pin!("vt-open/routed", opened, |c| c.vt_open(&routed(), &open));

    let close = VTCloseRequest {
        list_of_remote_vt_session_identifiers: vec![29, 30],
    };
    pin!("vt-close/local", Answer::Simple, |c| c
        .vt_close(PEER, &close));
    pin!("vt-close/routed", Answer::Simple, |c| c
        .vt_close(&routed(), &close));

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
    pin!("vt-data/local", partial.clone(), |c| c.vt_data(PEER, &data));
    pin!("vt-data/routed", partial, |c| c.vt_data(&routed(), &data));
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
    pin!("scpm/local", Answer::Simple, |c| c
        .subscribe_cov_property_multiple(PEER, &request));
    pin!("scpm/routed", Answer::Simple, |c| c
        .subscribe_cov_property_multiple(&routed(), &request));
}

/// The forms with their options left out: PrivateTransfer with no
/// parameters, TextMessage with no class, LifeSafetyOperation with no
/// object, and the SubscribeCOVPropertyMultiple cancellation.
#[tokio::test]
async fn forms_without_their_options() {
    let bare = PrivateTransferRequest {
        vendor_id: 555,
        service_number: 8,
        service_parameters: None,
    };
    let bare_ack = Answer::Complex(octets(|buf| {
        PrivateTransferAck {
            vendor_id: 555,
            service_number: 8,
            result_block: None,
        }
        .encode(buf)
    }));
    pin!("cpt-noparams/local", bare_ack, |c| c
        .confirmed_private_transfer(PEER, &bare));
    let classless = text_message(None);
    pin!("ctm-noclass/local", Answer::Simple, |c| c
        .confirmed_text_message(PEER, &classless));
    let every_object = LifeSafetyOperationRequest {
        requesting_process_identifier: 4,
        requesting_source: "operator".to_string(),
        request: LifeSafetyOperation::RESET,
        object_identifier: None,
    };
    pin!("lso-noobject/local", Answer::Simple, |c| c
        .life_safety_operation(PEER, &every_object));
    let cancel = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 12,
        issue_confirmed_notifications: false,
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: Vec::new(),
    };
    pin!("scpm-cancel/local", Answer::Simple, |c| c
        .subscribe_cov_property_multiple(PEER, &cancel));
}
