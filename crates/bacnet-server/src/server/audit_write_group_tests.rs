//! An inbound WriteGroup is audited as one WRITE per Channel it writes, the
//! operation Table 19-5 gives the service (#1318, Clause 19.6.5): the
//! Channel's Present_Value at the priority used, from the requester, with no
//! invoke ID (Table 19-4). A Channel write the mutation authorizer denies
//! makes no record (#1319).
//!
//! CH-1 and CH-2 serve channel 11 in group 27 and have no members, so the
//! records are the Channels' own. Device 30 is bound at `SOURCE`; a request
//! from `STRANGER` comes from an address no Device is bound to. Device 20 is
//! the Audit recipient, bound at `LOGGER`.
use super::*;
use crate::mutation::{MutationAuthorizer, MutationService, MutationTarget};
use bacnet_objects::channel::ChannelObject;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_types::constructed::BACnetAuditNotification;

const STRANGER: &[u8] = &[9];

async fn start() -> Fixture {
    let bindings = vec![
        DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
        DeviceBinding::local(oid(ObjectType::DEVICE, 30), SOURCE).unwrap(),
    ];
    let fixture = try_server(
        reporter(),
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        bindings,
    )
    .await
    .unwrap();
    for instance in [1, 2] {
        let mut channel = ChannelObject::new(instance, format!("CH-{instance}"), 11).unwrap();
        channel.set_control_groups(vec![27]).unwrap();
        fixture
            .server
            .database()
            .write()
            .await
            .add(Box::new(channel))
            .unwrap();
    }
    fixture
}

/// A WriteGroup for group 27 at priority 12 giving channel 11 REAL 5.0 at
/// priority 9.
fn write_group() -> Vec<u8> {
    let mut service = BytesMut::new();
    WriteGroupRequest {
        group_number: std::num::NonZeroU32::new(27).unwrap(),
        write_priority: 12,
        change_list: vec![GroupChannelValue {
            channel: 11,
            override_priority: Some(9),
            value: vec![0x44, 0x40, 0xA0, 0x00, 0x00],
        }],
        inhibit_delay: None,
    }
    .encode(&mut service)
    .unwrap();
    service.to_vec()
}

/// Hand `fixture`'s server the WriteGroup from `source_mac`, then let its
/// records go out.
async fn send(fixture: &Fixture, source_mac: &[u8]) {
    let received = bacnet_network::layer::ReceivedApdu {
        direct_response: None,
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(source_mac),
        ingress_network: None,
        source_network: None,
        link_layer_group: true,
        is_group: true,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: bacnet_transport::port::TransportProvenance::unverified(),
        reply_tx: None,
    };
    let request = UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::WRITE_GROUP,
        service_request: Bytes::from(write_group()),
    };
    let services = fixture.server.test_unconfirmed_services();
    BACnetServer::handle_unconfirmed_request(&services, request, &received).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

/// Every record sent so far.
fn records(fixture: &Fixture) -> Vec<BACnetAuditNotification> {
    notifications(&fixture.transport.sent)
        .into_iter()
        .flat_map(|request| request.notifications)
        .collect()
}

/// The record CH-`instance`'s write makes when `source` asks for it.
fn expected(instance: u32, source: BACnetRecipient) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: source,
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
        target_object: Some(oid(ObjectType::CHANNEL, instance)),
        target_property: Some(bacnet_types::constructed::AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }),
        target_priority: Some(9),
        target_value: Some(vec![0x44, 0x40, 0xA0, 0x00, 0x00]),
        current_value: Some(vec![0x00]),
        result: None,
    }
}

/// `records` without their target timestamps, which the clock decides.
fn untimed(records: Vec<BACnetAuditNotification>) -> Vec<BACnetAuditNotification> {
    records
        .into_iter()
        .map(|record| BACnetAuditNotification {
            target_timestamp: None,
            ..record
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn write_group_is_audited_once_per_channel_from_the_requester() {
    let fixture = start().await;
    send(&fixture, SOURCE).await;
    let device30 = BACnetRecipient::Device(oid(ObjectType::DEVICE, 30));
    assert_eq!(
        untimed(records(&fixture)),
        [expected(1, device30.clone()), expected(2, device30)]
    );

    // From an address no Device is bound to, the record names the address.
    fixture.transport.sent.lock().unwrap().clear();
    let channel = |instance| oid(ObjectType::CHANNEL, instance);
    for instance in [1, 2] {
        let mut db = fixture.server.database().write().await;
        // Back to NULL, so the current value is the same as before.
        db.get_mut(&channel(instance))
            .unwrap()
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Null,
                None,
            )
            .unwrap();
    }
    send(&fixture, STRANGER).await;
    let stranger = BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(STRANGER),
    });
    assert_eq!(
        untimed(records(&fixture)),
        [expected(1, stranger.clone()), expected(2, stranger)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_channel_write_the_authorizer_denies_makes_no_record() {
    let mut fixture = start().await;
    // Only CH-2 may be written; each decision sees an unconfirmed request.
    let authorizer: MutationAuthorizer = Arc::new(|context| {
        assert_eq!(context.invoke_id, None);
        assert_eq!(
            context.service_choice,
            MutationService::Unconfirmed(UnconfirmedServiceChoice::WRITE_GROUP)
        );
        let MutationTarget::WriteGroup(target) = &context.target else {
            panic!("a WriteGroup target, not {:?}", context.target);
        };
        target.channel.instance_number() == 2
    });
    fixture.server.config_mut().mutation_authorizer = Some(authorizer);
    send(&fixture, SOURCE).await;
    let device30 = BACnetRecipient::Device(oid(ObjectType::DEVICE, 30));
    assert_eq!(untimed(records(&fixture)), [expected(2, device30)]);
    let present_value = |instance| {
        let fixture = &fixture;
        async move {
            fixture
                .server
                .database()
                .read()
                .await
                .get(&oid(ObjectType::CHANNEL, instance))
                .unwrap()
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap()
        }
    };
    assert_eq!(present_value(1).await, PropertyValue::Null);
    assert_eq!(present_value(2).await, PropertyValue::Real(5.0));
    let counted = fixture.server.mutation_decision_counters().write_group;
    assert_eq!((counted.allow_total, counted.deny_total), (1, 1));
}
