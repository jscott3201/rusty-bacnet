//! COV notifications (#2303, PR 2): the server and the `Reporter` send the
//! same notifications for the same changes, answers and time.
//!
//! Each step goes to both sides at one paused instant. What the server
//! sends is compared with what the `Reporter`'s outbox hands out then:
//! unconfirmed APDUs octet for octet, confirmed ones with the invoke ID
//! left out (each side lends its own), with the link MAC and the routed
//! destination, as multisets. Design §7.2 asks for their order too; a set
//! it is, because the server starts a change's confirmed workers before
//! it sends its unconfirmed notifications (PR 2a), so their order across
//! subscriptions is its tasks' scheduling, not something the `Reporter`
//! has to copy. The test answers both as the subscriber
//! would, and between steps moves the clock to each of the `Reporter`'s
//! deadlines, so the server's retry timers and the `Reporter`'s fire
//! together. Each step also states how many notifications it calls for,
//! so a change both sides get wrong the same way fails too.
use super::*;
use bacnet_device_core::cov::{Delivery, Token};
use bacnet_device_core::report::{Bindings, Destination, InvokeIds, peer_address};
use bacnet_encoding::apdu::{ErrorPdu, SimpleAck};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetAddress;

/// The core side's invoke IDs: the next free one in turn, so a late
/// answer to an ended report doesn't match the one after it.
#[derive(Default)]
pub(super) struct Ids {
    taken: Vec<(BACnetAddress, u8)>,
    next: u8,
}

impl InvokeIds for Ids {
    fn take(&mut self, to: &BACnetAddress) -> Option<u8> {
        let id = (0..=u8::MAX)
            .map(|step| self.next.wrapping_add(step))
            .find(|id| !self.taken.contains(&(to.clone(), *id)))?;
        self.next = id.wrapping_add(1);
        self.taken.push((to.clone(), id));
        Some(id)
    }
    fn release(&mut self, to: &BACnetAddress, id: u8) {
        self.taken.retain(|taken| *taken != (to.clone(), id));
    }
}

struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}

/// A confirmed notification both sides sent: who answers it, and under
/// which invoke ID on each side.
pub(super) struct Outstanding {
    peer: SubscriberEndpoint,
    server: u8,
    core: u8,
    token: Token,
}

/// One notification as it went out: its link MAC, routed destination,
/// whether it is confirmed, and its APDU with any invoke ID zeroed.
type Wire = (Vec<u8>, Option<(u16, Vec<u8>)>, bool, Vec<u8>);

/// `apdu`'s invoke ID, zeroed in it when it is a confirmed request.
fn strip(apdu: &mut [u8]) -> Option<u8> {
    let confirmed = apdu[0] >> 4 == 0;
    confirmed.then(|| core::mem::take(&mut apdu[2]))
}

impl CovParity {
    /// Compare the notifications the server sent, `frames`, with all the
    /// `Reporter` hands out at `now`; unconfirmed ones are then sent on the
    /// core side, and confirmed ones wait for their answers. How many.
    pub(super) fn same_notifications(
        &mut self,
        what: &str,
        frames: &[test_transport::SentFrame],
        now: bacnet_time::Instant,
    ) -> usize {
        let mut server: Vec<(Wire, Option<u8>)> = frames
            .iter()
            .map(|frame| {
                let npdu = frame.decode_npdu();
                let mut apdu = npdu.payload.to_vec();
                let id = strip(&mut apdu);
                let to = npdu
                    .destination
                    .map(|to| (to.network, to.mac_address.to_vec()));
                ((frame.mac.to_vec(), to, id.is_some(), apdu), id)
            })
            .collect();
        let mut core = Vec::new();
        loop {
            let mut out = BytesMut::new();
            let reporter = &mut self.parity.reporter;
            let Some(sent) = reporter.pop(&NoBindings, &mut self.ids, now, &mut out) else {
                break;
            };
            let Destination::Subscriber(peer) = sent.to else {
                unreachable!("COV goes to subscribers")
            };
            let mut apdu = out.to_vec();
            let id = strip(&mut apdu);
            assert_eq!(id, sent.invoke_id, "{what}");
            if id.is_none() {
                reporter.delivered(sent.token, Delivery::Sent, &mut self.ids, now);
            }
            let to = peer
                .network
                .as_ref()
                .map(|to| (to.network, to.mac_address.to_vec()));
            core.push((
                (peer.mac.to_vec(), to, id.is_some(), apdu),
                (id, sent.token, peer),
            ));
        }
        server.sort_by(|a, b| a.0.cmp(&b.0));
        core.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            server.iter().map(|(wire, _)| wire).collect::<Vec<_>>(),
            core.iter().map(|(wire, _)| wire).collect::<Vec<_>>(),
            "{what}: the notifications, server then core"
        );
        for ((_, server), (_, (core, token, peer))) in server.iter().zip(core.iter()) {
            if let (Some(server), Some(core)) = (server, core) {
                // A resend is the same report again: keep one record.
                self.outstanding.retain(|out| out.token != *token);
                self.outstanding.push(Outstanding {
                    peer: peer.clone(),
                    server: *server,
                    core: *core,
                    token: *token,
                });
            }
        }
        frames.len()
    }

    /// Write `value` to `property` of `object` from `peer` on both; how many
    /// notifications the write called for.
    async fn write(
        &mut self,
        what: &str,
        peer: &SubscriberEndpoint,
        object: ObjectIdentifier,
        property: P,
        value: PropertyValue,
    ) -> usize {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let (reply, count) = self.exchange(what, peer, service, &body).await;
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{what}: {reply:?}");
        count
    }

    /// Answer the oldest outstanding confirmed notification on both sides,
    /// with a SimpleACK or, `refuse`d, an Error; how many notifications
    /// followed (the revisit an Ack owes).
    async fn answer(&mut self, what: &str, refuse: bool) -> usize {
        let out = self.outstanding.remove(0);
        let service = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
        let encode = |invoke_id| {
            let pdu = if refuse {
                Apdu::Error(ErrorPdu {
                    invoke_id,
                    service_choice: service,
                    error_class: ErrorClass::SERVICES,
                    error_code: ErrorCode::SERVICE_REQUEST_DENIED,
                    error_data: Bytes::new(),
                })
            } else {
                Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: service,
                })
            };
            let mut apdu = BytesMut::new();
            encode_apdu(&mut apdu, &pdu).unwrap();
            apdu.freeze()
        };
        let now = runtime_clock::core_now();
        let frames = self.deliver(&encode(out.server), &out.peer).await;
        let from = peer_address(&out.peer);
        let reporter = &mut self.parity.reporter;
        let core = encode(out.core);
        assert!(
            reporter.reply(&from, &core, &mut self.ids, now),
            "{what}: the core's report"
        );
        self.settle(now);
        self.same_notifications(what, &frames, now)
    }

    /// Run the `Reporter`'s work due at `now`, as the server's tasks run
    /// theirs.
    fn settle(&mut self, now: bacnet_time::Instant) {
        let parity = &mut self.parity;
        while parity
            .reporter
            .next_deadline()
            .is_some_and(|due| due <= now)
        {
            parity.reporter.on_deadline(&parity.db, &mut self.ids, now);
            if parity.reporter.next_deadline() == Some(now) {
                break;
            }
        }
    }

    /// Let `seconds` pass, stopping at each of the `Reporter`'s deadlines on
    /// the way; the notifications both sides sent meanwhile, compared.
    async fn advance(&mut self, what: &str, seconds: u64) -> usize {
        let end = runtime_clock::core_now() + bacnet_time::Duration::from_secs(seconds);
        let mut sent = 0;
        loop {
            let due = self
                .parity
                .reporter
                .next_deadline()
                .filter(|due| *due < end)
                .unwrap_or(end);
            let at = runtime_clock::core_clock().to_std(due).unwrap();
            tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await;
            // Let the server's tasks due at the same instant run.
            tokio::time::sleep(Duration::from_millis(1)).await;
            let frames = self.parity.sent.take();
            self.settle(due);
            sent += self.same_notifications(what, &frames, due);
            if due == end {
                return sent;
            }
        }
    }

    /// Read the local Device's `property` from both, required to agree.
    async fn read_list(&mut self, what: &str, property: P) {
        let device = oid(ObjectType::DEVICE, DEVICE);
        let apdu = read_property(1, device, property, None);
        let (server, core) = self.parity.exchange(apdu, 1476).await;
        assert!(
            matches!(core.as_ref().map(|apdu| apdu[0] >> 4), Some(3)),
            "{what}: a ComplexACK"
        );
        assert_eq!(server, core, "{what}: server, then core");
    }
}

fn staged_oid() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 9)
}

/// A custom object whose own Table 13-1 row reports Present_Stage, a
/// trigger, and Setpoint, a value that rides along; both writable.
#[derive(Debug)]
pub(super) struct Staged {
    stage: u64,
    setpoint: f32,
}

impl Staged {
    pub(super) fn new() -> Self {
        Self {
            stage: 1,
            setpoint: 20.0,
        }
    }
}

impl bacnet_objects::traits::BACnetObject for Staged {
    fn object_identifier(&self) -> ObjectIdentifier {
        staged_oid()
    }
    fn object_name(&self) -> &str {
        "Staged-9"
    }
    fn property_metadata(
        &self,
    ) -> std::borrow::Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        let served = [PV, P::STATUS_FLAGS, P::PRESENT_STAGE, P::SETPOINT];
        crate::test_rows::rows(&served, &[P::PRESENT_STAGE, P::SETPOINT])
    }
    fn read_property(&self, property: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        Ok(match property {
            P::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(staged_oid()),
            P::OBJECT_NAME => PropertyValue::CharacterString("Staged-9".into()),
            P::OBJECT_TYPE => PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw()),
            PV => PropertyValue::Real(0.0),
            P::STATUS_FLAGS => PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            },
            P::PRESENT_STAGE => PropertyValue::Unsigned(self.stage),
            P::SETPOINT => PropertyValue::Real(self.setpoint),
            _ => {
                return Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                ));
            }
        })
    }
    fn write_property(
        &mut self,
        property: P,
        _: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        match (property, value) {
            (P::PRESENT_STAGE, PropertyValue::Unsigned(stage)) => self.stage = stage,
            (P::SETPOINT, PropertyValue::Real(setpoint)) => self.setpoint = setpoint,
            _ => {
                return Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::WRITE_ACCESS_DENIED,
                ));
            }
        }
        Ok(())
    }
    fn cov(&self) -> Option<&dyn bacnet_objects::traits::CovReporting> {
        Some(self)
    }
}

impl bacnet_objects::traits::CovReporting for Staged {
    fn cov_reported_properties(
        &self,
    ) -> Option<&'static [bacnet_objects::traits::CovReportedProperty]> {
        use bacnet_objects::traits::CovReportedProperty::{Trigger, Value};
        Some(&[Trigger(P::PRESENT_STAGE), Value(P::SETPOINT)])
    }
}

const OOS: P = P::OUT_OF_SERVICE;
const PV: P = P::PRESENT_VALUE;

/// Changes under, at and over COV_Increment, Status_Flags alone, a
/// property subscription's own increment, and the Device's list before
/// and after an expiry, and as time passes.
#[tokio::test(start_paused = true)]
async fn unconfirmed_notifications_match() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let (a, b) = (direct(1), routed(2));
    let whole = subscribe(1, ai(), Some(false), Some(300));
    let (_, initial) = cov.exchange("SubscribeCOV", &a, SUBSCRIBE, &whole).await;
    assert_eq!(initial, 1, "the first notification");
    let own = subscribe_property(2, ai(), PV, None, Some(false), Some(300));
    let (_, initial) = cov
        .exchange("SubscribeCOVProperty", &b, PROPERTY, &own)
        .await;
    assert_eq!(initial, 1);
    let real = PropertyValue::Real;
    assert_eq!(
        cov.write("COV_Increment", &a, ai(), P::COV_INCREMENT, real(1.0))
            .await,
        0
    );
    let oos = PropertyValue::Boolean(true);
    assert_eq!(cov.write("Status_Flags alone", &a, ai(), OOS, oos).await, 2);
    // From 21.5: under both increments, then over the property's 0.5 only,
    // then exactly at the object's 1.0, then over it.
    assert_eq!(cov.write("under", &a, ai(), PV, real(21.75)).await, 0);
    assert_eq!(cov.write("over 0.5", &a, ai(), PV, real(22.1)).await, 1);
    assert_eq!(cov.write("at 1.0", &a, ai(), PV, real(22.5)).await, 1);
    assert_eq!(cov.write("over 1.0", &a, ai(), PV, real(24.0)).await, 2);

    cov.read_list("listed", P::ACTIVE_COV_SUBSCRIPTIONS).await;
    let short = subscribe(3, ai(), Some(false), Some(30));
    cov.exchange("thirty seconds", &a, SUBSCRIBE, &short).await;
    assert_eq!(cov.advance("time passes", 20).await, 0);
    cov.read_list("Time_Remaining after 20 s", P::ACTIVE_COV_SUBSCRIPTIONS)
        .await;
    assert_eq!(
        cov.write("Time_Remaining", &a, ai(), PV, real(30.0)).await,
        3
    );
    assert_eq!(cov.advance("past the expiry", 15).await, 0);
    cov.read_list("after the expiry", P::ACTIVE_COV_SUBSCRIPTIONS)
        .await;
    assert_eq!(cov.write("expired", &a, ai(), PV, real(40.0)).await, 2);
    cov.read_list("multiple", P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS)
        .await;
    cov.stop().await;
}

/// A change of a Table 13-1 trigger reports, with the value that rides
/// along; a change of that value alone doesn't.
#[tokio::test(start_paused = true)]
async fn a_table_13_1_trigger_reports() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let a = direct(1);
    let staged = staged_oid();
    let body = subscribe(1, staged, Some(false), Some(300));
    assert_eq!(cov.exchange("staged", &a, SUBSCRIBE, &body).await.1, 1);
    let setpoint = PropertyValue::Real(25.0);
    assert_eq!(
        cov.write("rides along", &a, staged, P::SETPOINT, setpoint)
            .await,
        0
    );
    let stage = PropertyValue::Unsigned(2);
    assert_eq!(
        cov.write("Present_Stage", &a, staged, P::PRESENT_STAGE, stage)
            .await,
        1
    );
    cov.stop().await;
}

/// A confirmed report acknowledged, refused (hold-off) and timed out
/// (resent, then held off), and a change held while one is out, reported
/// after the Ack.
#[tokio::test(start_paused = true)]
async fn confirmed_notifications_match() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let a = direct(1);
    let body = subscribe(1, ai(), Some(true), Some(600));
    assert_eq!(cov.exchange("confirmed", &a, SUBSCRIBE, &body).await.1, 1);
    let oos = PropertyValue::Boolean(true);
    let real = PropertyValue::Real;
    assert_eq!(cov.write("held", &a, ai(), OOS, oos).await, 0);
    assert_eq!(cov.answer("Ack, then the held change", false).await, 1);
    assert_eq!(cov.answer("Ack", false).await, 0);

    assert_eq!(cov.write("to refuse", &a, ai(), PV, real(30.0)).await, 1);
    assert_eq!(cov.answer("refused", true).await, 0);
    assert_eq!(cov.write("held off", &a, ai(), PV, real(40.0)).await, 0);
    assert_eq!(cov.advance("the hold-off", 13).await, 0);
    assert_eq!(cov.write("after it", &a, ai(), PV, real(50.0)).await, 1);
    // Unanswered: three resends, then a failure and its hold-off.
    assert_eq!(cov.advance("resent", 10).await, 3);
    assert_eq!(cov.advance("gave up", 3).await, 0);
    assert_eq!(
        cov.write("held off again", &a, ai(), PV, real(60.0)).await,
        0
    );
    cov.stop().await;
}

/// A renewal fences the outstanding report: its late Ack ends nothing, and
/// the renewal's first notification goes out at once.
#[tokio::test(start_paused = true)]
async fn a_renewal_fences_a_report() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let a = direct(1);
    let body = subscribe(1, ai(), Some(true), Some(600));
    assert_eq!(cov.exchange("confirmed", &a, SUBSCRIBE, &body).await.1, 1);
    let renewal = subscribe(1, ai(), Some(true), Some(300));
    assert_eq!(cov.exchange("renewal", &a, SUBSCRIBE, &renewal).await.1, 1);
    let late = cov.outstanding.remove(0);
    let ack = |invoke_id| {
        let mut apdu = BytesMut::new();
        let pdu = Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        });
        encode_apdu(&mut apdu, &pdu).unwrap();
        apdu.freeze()
    };
    let now = runtime_clock::core_now();
    let frames = cov.deliver(&ack(late.server), &late.peer).await;
    assert!(frames.is_empty(), "{frames:?}");
    let from = peer_address(&late.peer);
    let reporter = &mut cov.parity.reporter;
    assert!(!reporter.reply(&from, &ack(late.core), &mut cov.ids, now));
    assert_eq!(cov.answer("the renewal's Ack", false).await, 0);
    cov.stop().await;
}

/// A report whose server worker was aborted, and whose `Reporter` heard
/// nothing, ends at its deadline; the next change is reported.
///
/// The two sides aren't alike here: the `Reporter` reaches the deadline
/// only because the test stops calling it. A host that kept calling
/// `on_deadline` would see the report fail at its cutoff, before the
/// deadline, and hold off, as a server worker that kept running does.
#[tokio::test(start_paused = true)]
async fn an_abandoned_report_ends_at_its_deadline() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let a = direct(1);
    let body = subscribe(1, ai(), Some(true), Some(600));
    assert_eq!(cov.exchange("confirmed", &a, SUBSCRIBE, &body).await.1, 1);
    cov.parity.server.notification_transactions.abort_workers();
    cov.outstanding.clear();
    // Neither side hears of it again: the Reporter's host makes no call
    // until the report's deadline, 15 s on.
    tokio::time::sleep(Duration::from_secs(16)).await;
    assert!(cov.parity.sent.take().is_empty());
    let oos = PropertyValue::Boolean(true);
    assert_eq!(cov.write("after the deadline", &a, ai(), OOS, oos).await, 1);
    assert_eq!(cov.answer("its Ack", false).await, 0);
    cov.stop().await;
}
