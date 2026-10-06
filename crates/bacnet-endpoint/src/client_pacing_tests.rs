//! The endpoint client's minimum interval between confirmed requests to one
//! destination, on paused time (#1542). It is `BACnetClient`'s pacer, so
//! these check the wiring through the session: requests to one device are
//! spaced, other devices don't wait, zero changes nothing, and a retry keeps
//! the turn its request took.
use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, ComplexAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::sync::Mutex as StdMutex;
use tokio::time::{Duration, Instant};

type Sends = Arc<StdMutex<Vec<(Instant, MacAddr)>>>;

const A: &[u8] = &[0x0A];
const B: &[u8] = &[0x0B];

/// A link whose devices answer each ReadProperty `reply_after` it was sent,
/// from the MAC it went to, recording when each request left. The first
/// `unanswered` requests get no answer.
struct Devices {
    inbound_tx: mpsc::Sender<ReceivedNpdu>,
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    sends: Sends,
    unanswered: StdMutex<usize>,
    reply_after: Duration,
}

impl Devices {
    /// The answer to `request` from `mac`, unless the device stays silent.
    fn answer(&self, request: &[u8], mac: &[u8]) -> Result<Option<ReceivedNpdu>, Error> {
        let npdu = decode_npdu(Bytes::copy_from_slice(request))?;
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload)? else {
            return Ok(None);
        };
        {
            let mut unanswered = self.unanswered.lock().unwrap();
            if *unanswered > 0 {
                *unanswered -= 1;
                return Ok(None);
            }
        }
        let read = ReadPropertyRequest::decode(&request.service_request)?;
        let mut service_ack = BytesMut::new();
        ReadPropertyACK {
            object_identifier: read.object_identifier,
            property_identifier: read.property_identifier,
            property_array_index: read.property_array_index,
            property_value: vec![0x21, 42],
        }
        .encode(&mut service_ack);
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: service_ack.freeze(),
            }),
        )?;
        let mut reply = BytesMut::new();
        encode_npdu(
            &mut reply,
            &Npdu {
                payload: apdu.freeze(),
                ..Npdu::default()
            },
        )?;
        Ok(Some(ReceivedNpdu {
            direct_response: None,
            npdu: reply.freeze(),
            source_mac: MacAddr::from_slice(mac),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        }))
    }
}

impl TransportPort for Devices {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound_rx.take().expect("started once"))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sends
            .lock()
            .unwrap()
            .push((Instant::now(), MacAddr::from_slice(mac)));
        if let Some(reply) = self.answer(npdu, mac)? {
            let inbound = self.inbound_tx.clone();
            let delay = self.reply_after;
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let _ = inbound.send(reply).await;
            });
        }
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[0x01]
    }
}

fn devices(sends: &Sends, unanswered: usize, reply_after: Duration) -> Devices {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    Devices {
        inbound_tx,
        inbound_rx: Some(inbound_rx),
        sends: Arc::clone(sends),
        unanswered: StdMutex::new(unanswered),
        reply_after,
    }
}

/// A started client-only session over [`Devices`] with a 100 ms APDU
/// timeout, `retries` retries and the given interval.
async fn paced(
    interval_ms: u64,
    retries: u8,
    unanswered: usize,
    reply_after: Duration,
) -> (EndpointSession<Devices>, Sends) {
    let sends = Sends::default();
    let devices = devices(&sends, unanswered, reply_after);
    let config = SessionConfig {
        apdu_timeout_ms: 100,
        apdu_retries: retries,
        min_request_interval_ms: interval_ms,
        ..SessionConfig::default()
    };
    let mut session = EndpointSession::new(devices, SessionRole::ClientOnly, config).unwrap();
    session.start().await.unwrap();
    (session, sends)
}

async fn read(session: &EndpointSession<Devices>, mac: &[u8]) {
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    session
        .client()
        .unwrap()
        .read_property(mac, analog, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
}

/// When each request to `mac` left, relative to `start`.
fn sent_to(sends: &Sends, mac: &[u8], start: Instant) -> Vec<Duration> {
    sends
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, to)| to.as_slice() == mac)
        .map(|(at, _)| *at - start)
        .collect()
}

fn ms(values: &[u64]) -> Vec<Duration> {
    values.iter().copied().map(Duration::from_millis).collect()
}

#[tokio::test(start_paused = true)]
async fn endpoint_requests_to_one_destination_are_spaced_by_the_interval() {
    let (mut session, sends) = paced(50, 0, 0, Duration::ZERO).await;
    let start = Instant::now();
    for _ in 0..3 {
        read(&session, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 50, 100]));
    session.stop().await.unwrap();
}

/// The pause runs from the answer, as `BACnetClient`'s does: a device that
/// takes 30 ms to answer still gets the whole 50 ms before the next request.
#[tokio::test(start_paused = true)]
async fn an_endpoint_request_waits_the_interval_from_a_slow_answer() {
    let (mut session, sends) = paced(50, 0, 0, Duration::from_millis(30)).await;
    let start = Instant::now();
    for _ in 0..3 {
        read(&session, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 80, 160]));
    session.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn endpoint_requests_to_two_destinations_do_not_wait_on_each_other() {
    let (mut session, sends) = paced(50, 0, 0, Duration::ZERO).await;
    let start = Instant::now();
    tokio::join!(read(&session, A), read(&session, B));
    tokio::join!(read(&session, A), read(&session, B));
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 50]));
    assert_eq!(sent_to(&sends, B, start), ms(&[0, 50]));
    session.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_endpoint_interval_of_zero_sends_at_once() {
    let (mut session, sends) = paced(0, 0, 0, Duration::ZERO).await;
    let start = Instant::now();
    for _ in 0..3 {
        read(&session, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), [Duration::ZERO; 3]);
    session.stop().await.unwrap();
}

/// A retry goes when the APDU timeout runs out, not the interval: it keeps
/// the turn its request took. The next request waits the interval from the
/// answer to the retry.
#[tokio::test(start_paused = true)]
async fn an_endpoint_retry_is_not_held_back_by_the_interval() {
    let (mut session, sends) = paced(500, 1, 1, Duration::ZERO).await;
    let start = Instant::now();
    read(&session, A).await;
    read(&session, A).await;
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 100, 600]));
    session.stop().await.unwrap();
}

/// More than an hour is refused when the session is made, as
/// `BACnetClient`'s builders refuse it.
#[test]
fn an_endpoint_interval_past_an_hour_is_refused() {
    let new = |interval_ms| {
        let config = SessionConfig {
            min_request_interval_ms: interval_ms,
            ..SessionConfig::default()
        };
        let devices = devices(&Sends::default(), 0, Duration::ZERO);
        EndpointSession::new(devices, SessionRole::ClientOnly, config).map(drop)
    };
    let hour = bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS;
    assert!(new(hour).is_ok());
    assert!(matches!(new(hour + 1), Err(Error::Encoding(_))));
}
