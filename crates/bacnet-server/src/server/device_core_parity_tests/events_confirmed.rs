//! Confirmed event notifications (#2525): a destination that issues
//! confirmed notifications, which both sides send as ConfirmedEventNotification
//! requests, compared by their service parameters, not by count.
//!
//! The invoke ID is each host's own, so it is left out of the comparison and
//! everything else in the APDU, the header's flags and limits and the
//! service parameters, is compared octet for octet; the parameters of each
//! confirmed notification are also decoded and checked field by field.
//!
//! Order. Of the unconfirmed notifications one change sends, the order is
//! defined: the Recipient_List's, from both, compared in order
//! (`same_events`). Of the confirmed ones it is not: the server sends each
//! from a transaction worker of its own, so the order across workers, and
//! between a confirmed notification and an unconfirmed one, is its
//! scheduling; they compare as a set, by recipient. Each recipient's own
//! sequence is ordered, in the steps that follow one another.
use super::device::parameters;
use super::*;
use bacnet_encoding::apdu::SimpleAck;
use bacnet_types::enums::NotifyType;

/// The two destinations that issue confirmed notifications.
const FIRST: [u8; 6] = [10, 0, 9, 11, 0xBA, 0xC0];
const SECOND: [u8; 6] = [10, 0, 9, 12, 0xBA, 0xC0];

/// Each destination's own process identifier: its MAC's fourth octet.
fn confirming(mac: [u8; 6]) -> BACnetDestination {
    let mut destination = recipients().swap_remove(0);
    destination.recipient = BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&mac),
    });
    destination.process_identifier = u32::from(mac[3]);
    destination.issue_confirmed_notifications = true;
    destination
}

/// Time_Delay 0, and four destinations: two confirming, between two that
/// don't.
fn mixed(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let [always, _, _, midday, _] = <[_; 5]>::try_from(recipients()).unwrap();
    let destinations = vec![confirming(FIRST), always, confirming(SECOND), midday];
    database_with(identity, alarm_value(0, destinations))
}

impl EventParity {
    /// Answer each outstanding confirmed notification with a SimpleACK, on
    /// the server from its recipient and on the core's `Reporter`, at
    /// `millis` after the start; how many notifications followed.
    async fn answer_all(&mut self, what: &str, millis: u64) -> usize {
        let now = self.instant(millis);
        self.sleep_until(now).await;
        let outstanding = std::mem::take(&mut self.outstanding);
        assert!(!outstanding.is_empty(), "{what}: nothing to answer");
        let ack = |invoke_id| {
            let pdu = Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            });
            let mut apdu = BytesMut::new();
            encode_apdu(&mut apdu, &pdu).unwrap();
            apdu.freeze()
        };
        let mut frames = Vec::new();
        for out in &outstanding {
            let mut npdu = BytesMut::new();
            let message = Npdu {
                payload: ack(out.server),
                ..Npdu::default()
            };
            encode_npdu(&mut npdu, &message).unwrap();
            self.parity
                .tx
                .send(ReceivedNpdu {
                    direct_response: None,
                    npdu: npdu.freeze(),
                    source_mac: MacAddr::from_slice(&out.mac),
                    link_layer_group: false,
                    data_attributes: Vec::new(),
                    provenance: TransportProvenance::unverified(),
                    reply_tx: None,
                })
                .await
                .unwrap();
            let from = BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(&out.mac),
            };
            let answered = self
                .parity
                .reporter
                .reply(&from, &ack(out.core), &mut self.ids, now);
            assert!(answered, "{what}: the core's notification");
        }
        // The paused clock moves only once every server task is idle.
        tokio::time::sleep(Duration::from_millis(20)).await;
        frames.extend(self.parity.sent.take());
        self.settle(now);
        self.same_events(what, &frames, now, None)
    }

    /// The confirmed notifications among `compared`, taken: each one's link
    /// MAC and decoded service parameters.
    fn confirmed_taken(
        &mut self,
    ) -> Vec<(Vec<u8>, bacnet_types::constructed::EventNotificationRequest)> {
        let compared = std::mem::take(&mut self.compared);
        let confirmed = compared.iter().filter(|wire| wire.2[0] >> 4 == 0);
        confirmed
            .map(|wire| (wire.0.clone(), parameters(&wire.2)))
            .collect()
    }
}

/// An alarm and its return, reported to two confirming destinations and two
/// that don't: each confirmed notification's parameters are those of its
/// recipient and the transition, and both sides send the same, each answered.
#[tokio::test(start_paused = true)]
async fn confirmed_notifications_match_by_their_parameters() {
    let mut both = EventParity::start(mixed).await;
    assert_eq!(both.value("over the limit", 300, 90.0).await, 4);
    assert_eq!(both.outstanding.len(), 2, "two are confirmed");
    let mut confirmed = both.confirmed_taken();
    confirmed.sort_by_key(|sent| sent.0.clone());
    let macs: Vec<_> = confirmed.iter().map(|sent| sent.0.clone()).collect();
    assert_eq!(macs, [FIRST.to_vec(), SECOND.to_vec()]);
    for (mac, notification) in &confirmed {
        assert_eq!(notification.process_identifier, u32::from(mac[3]));
        assert_eq!(notification.to_state, EventState::HIGH_LIMIT);
        assert_eq!(notification.from_state, EventState::NORMAL);
        assert_eq!(notification.notify_type, NotifyType::ALARM);
        assert!(
            notification.ack_required,
            "TO_OFFNORMAL needs acknowledging"
        );
        assert_eq!(notification.priority, 100);
        assert_eq!(notification.event_object_identifier, av());
    }
    // Answered, they are done: nothing follows an Ack.
    assert_eq!(both.answer_all("answered", 400).await, 0);
    assert!(both.outstanding.is_empty());
    // Back to normal after Time_Delay_Normal: four again, each recipient's
    // second, the confirmed ones' TO_NORMAL needing no acknowledgement.
    assert_eq!(both.value("back under", 4_500, 50.0).await, 0);
    assert_eq!(both.run_to("back to normal", 5_550).await, 4);
    let mut back = both.confirmed_taken();
    back.sort_by_key(|sent| sent.0.clone());
    assert_eq!(back.len(), 2);
    for (mac, notification) in &back {
        assert_eq!(notification.process_identifier, u32::from(mac[3]));
        assert_eq!(notification.to_state, EventState::NORMAL);
        assert_eq!(notification.from_state, EventState::HIGH_LIMIT);
        assert!(!notification.ack_required);
        assert_eq!(notification.priority, 255);
    }
    assert_eq!(both.answer_all("answered", 5_600).await, 0);
}

/// Unanswered, a confirmed notification is sent again, the same by its
/// parameters, to the same recipient, from both, at the server's retry
/// interval, and answered it ends.
#[tokio::test(start_paused = true)]
async fn an_unanswered_confirmed_notification_is_sent_again_alike() {
    let mut both = EventParity::start(mixed).await;
    assert_eq!(both.value("over the limit", 300, 90.0).await, 4);
    both.compared.clear();
    // No answer: the retry timeout later each side sends the two again.
    assert_eq!(both.run_to("resent", 3_500).await, 2);
    let again = both.confirmed_taken();
    assert_eq!(again.len(), 2);
    assert!(
        again
            .iter()
            .all(|sent| sent.1.to_state == EventState::HIGH_LIMIT)
    );
    assert_eq!(both.answer_all("answered", 3_600).await, 0);
}
