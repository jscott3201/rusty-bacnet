//! Wire harness for server COV tests: an in-memory transport that records
//! sent APDUs and moves a shared Device clock at chosen points, plus request,
//! subscription, notification and subscriber-response helpers.
use super::*;
use crate::server::test_transport::{SentFrame, TestTransport};
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::{
    COVNotificationRequest, SubscribeCOVPropertyRequest, SubscribeCOVRequest,
};
use bacnet_services::cov_multiple::{
    COVNotificationMultipleRequest, COVNotificationValue, COVReference,
    COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::{Date, Time};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex as StdMutex;
use tokio::sync::mpsc;

pub(super) const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
pub(super) const SF: PropertyIdentifier = PropertyIdentifier::STATUS_FLAGS;
pub(super) const PEER: [u8; 6] = [10, 0, 0, 5, 0xBA, 0xC0];
/// A multicast address, which the harness link reports as a group
/// destination that isn't its broadcast (#1493).
pub(super) const GROUP: [u8; 6] = [224, 0, 0, 1, 0xBA, 0xC0];

pub(super) fn at(second: u8) -> ClockFrame {
    assert!(second < 60, "an invalid Device clock captures nothing");
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 29,
            day_of_week: 2,
        },
        local_time: Time {
            hour: 15,
            minute: 0,
            second,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

pub(super) fn time(second: u8) -> Time {
    at(second).local_time
}

#[derive(Clone)]
pub(super) struct SharedClock(pub(super) Arc<StdMutex<ClockFrame>>);

impl ClockReader for SharedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(*self.0.lock().unwrap())
    }
}

pub(super) type Frames = Arc<StdMutex<Vec<Apdu>>>;

/// What to do to one coming COV-multiple notification.
enum PlanAction {
    /// Hold it in the transport until a permit arrives.
    Hold(Arc<tokio::sync::Semaphore>),
    /// Fail its send.
    Fail,
    /// Disable initiation right after sending it.
    Disable(Arc<CommState>),
}

/// An action on the COV-multiple notification after `after` more of them.
struct NotificationPlan {
    after: usize,
    action: PlanAction,
}

type Plan = Arc<StdMutex<Option<NotificationPlan>>>;

fn is_cov_multiple(apdu: &Apdu) -> bool {
    match apdu {
        Apdu::UnconfirmedRequest(request) => {
            request.service_choice
                == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
        }
        Apdu::ConfirmedRequest(request) => {
            request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
        }
        _ => false,
    }
}

/// Send side of the harness link: records unicast APDUs and moves the shared
/// Device clock at chosen points.
#[derive(Clone)]
struct ClockLink {
    frames: Frames,
    clock: SharedClock,
    /// Device time once a SimpleACK or Error response has been sent.
    after_ack: Arc<StdMutex<Option<ClockFrame>>>,
    /// Device time once a broadcast (an event notification) has been sent.
    after_broadcast: Arc<StdMutex<Option<ClockFrame>>>,
    fail_notifications: Arc<AtomicBool>,
    plan: Plan,
}

impl ClockLink {
    /// The plan's action if `apdu` is the COV-multiple notification it names.
    fn planned(&self, apdu: &Apdu) -> Option<PlanAction> {
        if !is_cov_multiple(apdu) {
            return None;
        }
        let mut plan = self.plan.lock().unwrap();
        match plan.as_mut() {
            Some(pending) if pending.after > 0 => {
                pending.after -= 1;
                None
            }
            Some(_) => plan.take().map(|due| due.action),
            None => None,
        }
    }

    async fn send(self, frame: SentFrame) -> Result<(), Error> {
        if frame.broadcast {
            if let Some(next) = self.after_broadcast.lock().unwrap().take() {
                *self.clock.0.lock().unwrap() = next;
            }
            return Ok(());
        }
        let apdu = frame.apdu();
        match &apdu {
            Apdu::SimpleAck(_) | Apdu::Error(_) => {
                if let Some(next) = self.after_ack.lock().unwrap().take() {
                    *self.clock.0.lock().unwrap() = next;
                }
            }
            Apdu::UnconfirmedRequest(_) | Apdu::ConfirmedRequest(_)
                if self.fail_notifications.load(Ordering::Acquire) =>
            {
                return Err(Error::Encoding("injected notification send failure".into()));
            }
            _ => {}
        }
        match self.planned(&apdu) {
            Some(PlanAction::Hold(release)) => {
                release.acquire().await.unwrap().forget();
                self.frames.lock().unwrap().push(apdu);
            }
            Some(PlanAction::Fail) => {
                return Err(Error::Encoding("injected notification send failure".into()));
            }
            Some(PlanAction::Disable(comm_state)) => {
                self.frames.lock().unwrap().push(apdu);
                comm_state.set_for_test(DccState::DisableInitiation);
            }
            None => self.frames.lock().unwrap().push(apdu),
        }
        Ok(())
    }
}

pub(super) struct Harness {
    pub(super) server: BACnetServer<TestTransport>,
    pub(super) tx: mpsc::Sender<ReceivedNpdu>,
    pub(super) frames: Frames,
    pub(super) clock: SharedClock,
    pub(super) after_ack: Arc<StdMutex<Option<ClockFrame>>>,
    pub(super) after_broadcast: Arc<StdMutex<Option<ClockFrame>>>,
    pub(super) fail_notifications: Arc<AtomicBool>,
    plan: Plan,
    pub(super) invoke_id: u8,
    /// Max-APDU-length-accepted the subscriber advertises in its requests.
    pub(super) request_max_apdu: u16,
    /// COV increment the harness's PV references subscribe with.
    pub(super) pv_increment: f32,
    /// Invoke ID and service of the last confirmed notification taken.
    last_confirmed: StdMutex<Option<(u8, ConfirmedServiceChoice)>>,
    /// Every confirmed notification taken. Invoke IDs rotate, so a byte-equal
    /// request with the same ID later is a retry, never a new report.
    taken: StdMutex<Vec<ConfirmedRequestPdu>>,
}

impl Harness {
    pub(super) async fn start(config: ServerConfig) -> Self {
        Self::start_with(config, |_| {}).await
    }

    pub(super) async fn start_with(
        mut config: ServerConfig,
        extend: impl FnOnce(&mut ObjectDatabase),
    ) -> Self {
        config.device = bacnet_objects::DeviceIdentity::new(856, 0)
            .unwrap()
            .with_max_apdu(config.device.max_apdu_length())
            .unwrap();
        let (tx, rx) = mpsc::channel(16);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(856, 0)
                    .expect("valid Device identity")
                    .with_name("Timed COV Device")
                    .with_max_apdu(
                        u16::try_from(u32::from(config.device.max_apdu_length()))
                            .expect("Device APDU fits u16"),
                    )
                    .expect("valid Device APDU"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS);
        db.add(Box::new(av.unwrap())).unwrap();
        extend(&mut db);
        let frames = Arc::new(StdMutex::new(Vec::new()));
        let clock = SharedClock(Arc::new(StdMutex::new(at(0))));
        let after_ack = Arc::new(StdMutex::new(None));
        let after_broadcast = Arc::new(StdMutex::new(None));
        let fail_notifications = Arc::new(AtomicBool::new(false));
        let plan: Plan = Arc::default();
        let link = ClockLink {
            frames: Arc::clone(&frames),
            clock: clock.clone(),
            after_ack: Arc::clone(&after_ack),
            after_broadcast: Arc::clone(&after_broadcast),
            fail_notifications: Arc::clone(&fail_notifications),
            plan: Arc::clone(&plan),
        };
        let transport = TestTransport::builder()
            .local_mac(&[10, 0, 0, 2, 0xBA, 0xC0])
            .group_mac(&GROUP)
            .inbound(rx)
            .on_send(move |frame| link.clone().send(frame))
            .build();
        let server = BACnetServer::start(config, db, transport).await.unwrap();
        // Start installs the system clock; replace it with the test clock.
        server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(clock.clone())));
        Self {
            server,
            tx,
            frames,
            clock,
            after_ack,
            after_broadcast,
            fail_notifications,
            plan,
            invoke_id: 0,
            request_max_apdu: 1476,
            pv_increment: 0.5,
            last_confirmed: StdMutex::new(None),
            taken: StdMutex::new(Vec::new()),
        }
    }

    pub(super) fn set_clock(&self, second: u8) {
        *self.clock.0.lock().unwrap() = at(second);
    }

    /// Put `object` in place of the stored object with its identifier, as an
    /// application holding new input would, then run the server's COV fanout
    /// for it as a committed write does.
    pub(super) async fn replace_and_fan_out(&self, object: Box<dyn BACnetObject>) {
        let oid = object.object_identifier();
        self.server.database().write().await.add(object).unwrap();
        let server = &self.server;
        BACnetServer::<TestTransport>::fire_cov_notifications(
            &super::cov_notify_context::CovNotifyContext {
                db: &server.db,
                network: server.test_network(),
                cov_table: &server.cov_table,
                cov_in_flight: &server.cov_in_flight,
                notification_transactions: &server.notification_transactions,
                comm_state: &server.comm_state,
                config: &server.config,
            },
            &oid,
        )
        .await;
    }

    fn plan(&self, after: usize, action: PlanAction) {
        *self.plan.lock().unwrap() = Some(NotificationPlan { after, action });
    }

    /// Hold the COV-multiple notification after `after` more in the
    /// transport until a permit is added to the returned semaphore.
    pub(super) fn hold_notification(&self, after: usize) -> Arc<tokio::sync::Semaphore> {
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        self.plan(after, PlanAction::Hold(Arc::clone(&release)));
        release
    }

    /// Fail the send of the COV-multiple notification after `after` more.
    pub(super) fn fail_notification(&self, after: usize) {
        self.plan(after, PlanAction::Fail);
    }

    /// Disable initiation right after sending the COV-multiple notification
    /// after `after` more, as a DeviceCommunicationControl would.
    pub(super) fn disable_after_notification(&self, after: usize) {
        self.plan(
            after,
            PlanAction::Disable(Arc::clone(&self.server.comm_state)),
        );
    }

    /// Deliver a confirmed request whose response goes out through the
    /// transport, as for a B/IP peer.
    pub(super) async fn request(&mut self, service_choice: ConfirmedServiceChoice, body: BytesMut) {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let mut payload = BytesMut::new();
        encode_apdu(
            &mut payload,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: self.request_max_apdu,
                invoke_id: self.invoke_id,
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
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&PEER),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
    }

    pub(super) async fn subscribe(&mut self, confirmed: bool) {
        self.subscribe_specs(confirmed, vec![(av1(), vec![(PV, true)])])
            .await;
    }

    /// One context: `(object, [(property, timestamped)])` specifications.
    pub(super) async fn subscribe_specs(
        &mut self,
        confirmed: bool,
        specs: Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, bool)>)>,
    ) {
        self.subscribe_with_delay(confirmed, specs, 10).await;
    }

    /// [`Self::subscribe_specs`] with a chosen Max_Notification_Delay.
    pub(super) async fn subscribe_with_delay(
        &mut self,
        confirmed: bool,
        specs: Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, bool)>)>,
        max_notification_delay: u32,
    ) {
        self.subscribe_process(856, confirmed, specs, Some(max_notification_delay))
            .await;
    }

    /// Cancel `specs` of the harness's usual context.
    pub(super) async fn cancel_specs(
        &mut self,
        confirmed: bool,
        specs: Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, bool)>)>,
    ) {
        self.subscribe_process(856, confirmed, specs, None).await;
    }

    /// SubscribeCOVPropertyMultiple for `process`: a 300 s subscription with
    /// `max_notification_delay`, or a cancellation when it is `None`.
    pub(super) async fn subscribe_process(
        &mut self,
        process: u32,
        confirmed: bool,
        specs: Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, bool)>)>,
        max_notification_delay: Option<u32>,
    ) {
        let mut body = BytesMut::new();
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: process,
            issue_confirmed_notifications: confirmed,
            lifetime: max_notification_delay.map(|_| 300),
            max_notification_delay,
            list_of_cov_subscription_specifications: specs
                .into_iter()
                .map(|(object, references)| COVSubscriptionSpecification {
                    monitored_object_identifier: object,
                    list_of_cov_references: references
                        .into_iter()
                        .map(|(property, timestamped)| COVReference {
                            monitored_property: PropertyReference {
                                property_identifier: property,
                                property_array_index: None,
                            },
                            cov_increment: (property == PV
                                && object.object_type() == ObjectType::ANALOG_VALUE)
                                .then_some(self.pv_increment),
                            timestamped,
                        })
                        .collect(),
                })
                .collect(),
        }
        .encode(&mut body)
        .unwrap();
        self.request(
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            body,
        )
        .await;
    }

    /// DeviceCommunicationControl without a password, for a server configured
    /// with the legacy permissive DCC policy.
    pub(super) async fn dcc(
        &mut self,
        enable_disable: bacnet_types::enums::EnableDisable,
        minutes: Option<u16>,
    ) {
        let mut body = BytesMut::new();
        bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: minutes,
            enable_disable,
            password: None,
        }
        .encode(&mut body)
        .unwrap();
        self.request(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL, body)
            .await;
    }

    /// WriteProperty PV at priority 8; the clock moves to `after_ack` when the
    /// SimpleACK is sent, before the server prepares its COV notification.
    pub(super) async fn write_pv(&mut self, value: f32, after_ack: u8) {
        *self.after_ack.lock().unwrap() = Some(at(after_ack));
        let mut encoded = BytesMut::new();
        bacnet_encoding::primitives::encode_property_value(
            &mut encoded,
            &PropertyValue::Real(value),
        )
        .unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: av1(),
            property_identifier: PV,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: Some(8),
        }
        .encode(&mut body)
        .unwrap();
        self.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
            .await;
    }

    pub(super) async fn write_local(&self, value: f32) {
        self.write_local_to(av1(), value).await;
    }

    pub(super) async fn write_local_to(&self, object: ObjectIdentifier, value: f32) {
        self.server
            .write_local(
                &object,
                PV,
                None,
                PropertyValue::Real(value),
                Some(8),
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
    }

    /// Whether `apdu` retries a confirmed notification already taken.
    fn is_retry(&self, apdu: &Apdu) -> bool {
        matches!(apdu, Apdu::ConfirmedRequest(request)
            if self.taken.lock().unwrap().iter().any(|taken| taken == request))
    }

    /// Wait for the next new notification carrying either service choice,
    /// discarding retries of those already taken and leaving other frames.
    async fn next_notification(
        &self,
        confirmed: ConfirmedServiceChoice,
        unconfirmed: UnconfirmedServiceChoice,
        what: &str,
    ) -> Bytes {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let next = {
                    let mut frames = self.frames.lock().unwrap();
                    frames.retain(|apdu| !self.is_retry(apdu));
                    let at = frames.iter().position(|apdu| match apdu {
                        Apdu::UnconfirmedRequest(request) => request.service_choice == unconfirmed,
                        Apdu::ConfirmedRequest(request) => request.service_choice == confirmed,
                        _ => false,
                    });
                    at.map(|at| frames.remove(at))
                };
                match next {
                    Some(Apdu::UnconfirmedRequest(request)) => return request.service_request,
                    Some(Apdu::ConfirmedRequest(request)) => {
                        *self.last_confirmed.lock().unwrap() =
                            Some((request.invoke_id, request.service_choice));
                        let body = request.service_request.clone();
                        self.taken.lock().unwrap().push(request);
                        return body;
                    }
                    // Sleep rather than spin, so paused-time tests can advance.
                    _ => tokio::time::sleep(Duration::from_millis(1)).await,
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{what} notification"))
    }

    /// Wait for the next COV-multiple notification.
    pub(super) async fn notification(&self) -> COVNotificationMultipleRequest {
        let body = self
            .next_notification(
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
                "COV-multiple",
            )
            .await;
        COVNotificationMultipleRequest::decode(&body).unwrap()
    }

    /// Wait for the next ordinary (SubscribeCOV) notification.
    pub(super) async fn cov_notification(&self) -> COVNotificationRequest {
        let body = self
            .next_notification(
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                "COV",
            )
            .await;
        COVNotificationRequest::decode(&body).unwrap()
    }

    /// SubscribeCOVProperty for one property of AV-1.
    pub(super) async fn subscribe_cov_property(&mut self, property: PropertyIdentifier) {
        let mut body = BytesMut::new();
        SubscribeCOVPropertyRequest {
            subscriber_process_identifier: 890,
            monitored_object_identifier: av1(),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
            monitored_property_identifier: property,
            monitored_property_array_index: None,
            cov_increment: None,
        }
        .encode(&mut body)
        .unwrap();
        self.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
            .await;
    }

    /// SubscribeCOV for AV-1: an ordinary, untimestamped subscription.
    pub(super) async fn subscribe_cov(&mut self) {
        let mut body = BytesMut::new();
        SubscribeCOVRequest {
            subscriber_process_identifier: 889,
            monitored_object_identifier: av1(),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        }
        .encode(&mut body)
        .unwrap();
        self.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body)
            .await;
    }

    /// No new notification within 50 ms; retries of taken ones don't count.
    pub(super) async fn no_notification(&self) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !self.frames.lock().unwrap().iter().any(|apdu| match apdu {
                _ if self.is_retry(apdu) => false,
                Apdu::UnconfirmedRequest(_) => true,
                Apdu::ConfirmedRequest(request) => matches!(
                    request.service_choice,
                    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
                        | ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
                ),
                _ => false,
            }),
            "no notification expected"
        );
    }

    /// Invoke ID and service of the last confirmed notification taken, for a
    /// later answer. Each notification is answered at most once.
    pub(super) fn take_confirmed(&self) -> (u8, ConfirmedServiceChoice) {
        self.last_confirmed
            .lock()
            .unwrap()
            .take()
            .expect("a confirmed notification to answer")
    }

    /// Deliver an answer from the subscriber.
    pub(super) async fn respond(&self, answer: Apdu) {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &answer).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&PEER),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
    }

    /// In paused time, return once the server has run all the work it has
    /// ready, such as handling an Ack and the follow-up fanout it owes.
    pub(super) async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    /// Acknowledge the last confirmed notification taken.
    pub(super) async fn ack(&self) {
        self.ack_request(self.take_confirmed()).await;
    }

    /// Acknowledge one confirmed notification.
    pub(super) async fn ack_request(
        &self,
        (invoke_id, service_choice): (u8, ConfirmedServiceChoice),
    ) {
        self.respond(Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice,
        }))
        .await;
    }

    /// Answer the last confirmed notification taken with an Error.
    pub(super) async fn reject(&self) {
        let (invoke_id, service_choice) = self.take_confirmed();
        self.respond(Apdu::Error(bacnet_encoding::apdu::ErrorPdu {
            invoke_id,
            service_choice,
            error_class: bacnet_types::enums::ErrorClass::SERVICES,
            error_code: bacnet_types::enums::ErrorCode::OTHER,
            error_data: Bytes::new(),
        }))
        .await;
    }

    /// Wait until every confirmed notification worker has finished.
    pub(super) async fn workers_idle(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.server.notification_transactions.active_count() > 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("confirmed notification workers finished");
    }
}

pub(super) fn av1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

pub(super) fn real(value: f32) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &PropertyValue::Real(value))
        .unwrap();
    encoded.to_vec()
}

/// Wait for the SimpleACK or Error answering the last request sent, take it,
/// and return the error code of an Error.
pub(super) async fn response(h: &Harness) -> Result<(), ErrorCode> {
    error_response(h).await.map_err(|error| error.error_code)
}

/// Wait for the SimpleACK or Error PDU answering the last request sent, take
/// it, and return the whole Error PDU.
pub(super) async fn error_response(h: &Harness) -> Result<(), bacnet_encoding::apdu::ErrorPdu> {
    let invoke_id = h.invoke_id;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let answer = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames.iter().position(|apdu| match apdu {
                    Apdu::SimpleAck(ack) => ack.invoke_id == invoke_id,
                    Apdu::Error(error) => error.invoke_id == invoke_id,
                    _ => false,
                });
                at.map(|at| frames.remove(at))
            };
            match answer {
                Some(Apdu::SimpleAck(_)) => return Ok(()),
                Some(Apdu::Error(error)) => return Err(error),
                _ => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    })
    .await
    .expect("a response to the last request")
}

/// `(property, value bytes, time)` rows of the single monitored object.
pub(super) fn rows(
    notification: &COVNotificationMultipleRequest,
) -> Vec<(PropertyIdentifier, Vec<u8>, Option<Time>)> {
    assert_eq!(notification.list_of_cov_notifications.len(), 1);
    let item = &notification.list_of_cov_notifications[0];
    assert_eq!(item.monitored_object_identifier, av1());
    item.list_of_values
        .iter()
        .map(
            |COVNotificationValue {
                 property_identifier,
                 value,
                 time_of_change,
                 ..
             }| { (*property_identifier, value.clone(), *time_of_change) },
        )
        .collect()
}

pub(super) fn pv_rows(
    notification: &COVNotificationMultipleRequest,
) -> Vec<(Vec<u8>, Option<Time>)> {
    rows(notification)
        .into_iter()
        .filter(|(property, _, _)| *property == PV)
        .map(|(_, value, time)| (value, time))
        .collect()
}

pub(super) fn envelope(notification: &COVNotificationMultipleRequest) -> Option<(Date, Time)> {
    notification.timestamp
}

pub(super) fn high_limit_alarm(db: &mut ObjectDatabase) {
    let mut nc = bacnet_objects::notification_class::NotificationClass::new(0, "NC-0").unwrap();
    nc.add_destination(super::event_notifications_tests::local_broadcast_destination())
        .unwrap();
    db.add(Box::new(nc)).unwrap();
    let object = db.get_mut(&av1()).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 0.0),
        (PropertyIdentifier::DEADBAND, 1.0),
    ] {
        object
            .write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    for (property, unused_bits, bits) in [
        (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
        (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
    ] {
        object
            .write_property(
                property,
                None,
                PropertyValue::BitString {
                    unused_bits,
                    data: vec![bits],
                },
                None,
            )
            .unwrap();
    }
}
