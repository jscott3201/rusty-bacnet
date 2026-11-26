//! The session's client's minimum interval between confirmed requests to
//! one destination, on paused time (#1542). The pacer is `BACnetClient`'s
//! own, so these check that the session's configuration reaches it:
//! requests to one device are spaced, other devices don't wait, zero
//! changes nothing, and a retry keeps the turn its request took.
use super::*;
use bacnet_encoding::apdu::{ComplexAck, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendMode, TestTransport};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::sync::Mutex as StdMutex;
use tokio::time::{Duration, Instant};

type Sends = Arc<StdMutex<Vec<(Instant, MacAddr)>>>;
type Client = Arc<bacnet_client::client::BACnetClient<TestTransport>>;

const A: &[u8] = &[0x0A];
const B: &[u8] = &[0x0B];

/// A link whose devices answer each ReadProperty `reply_after` it was sent,
/// from the MAC it went to, recording when each request left. The first
/// `unanswered` requests get no answer.
fn devices(sends: &Sends, unanswered: usize, reply_after: Duration) -> TestTransport {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    let (sends, unanswered) = (Arc::clone(sends), StdMutex::new(unanswered));
    TestTransport::builder()
        .inbound(inbound_rx)
        .broadcast(SendMode::Ignore)
        .on_send(move |frame| {
            sends
                .lock()
                .unwrap()
                .push((Instant::now(), MacAddr::from_slice(&frame.mac)));
            let result = answer(&unanswered, &frame.npdu, &frame.mac).map(|reply| {
                if let Some(reply) = reply {
                    let inbound = inbound_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(reply_after).await;
                        let _ = inbound.send(reply).await;
                    });
                }
            });
            async move { result }
        })
        .build()
}

/// The answer to `request` from `mac`, unless the device stays silent.
fn answer(
    unanswered: &StdMutex<usize>,
    request: &[u8],
    mac: &[u8],
) -> Result<Option<ReceivedNpdu>, Error> {
    let npdu = decode_npdu(Bytes::copy_from_slice(request))?;
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload)? else {
        return Ok(None);
    };
    {
        let mut unanswered = unanswered.lock().unwrap();
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
    .encode(&mut service_ack)
    .unwrap();
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

/// A started client-only session over [`devices`] with a 100 ms APDU
/// timeout, `retries` retries and the given interval, and its client.
async fn paced(
    interval_ms: u64,
    retries: u8,
    unanswered: usize,
    reply_after: Duration,
) -> (EndpointSession<TestTransport>, Client, Sends) {
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
    let client = Arc::new(session.client().unwrap());
    (session, client, sends)
}

async fn try_read(client: &Client, mac: &[u8]) -> Result<(), Error> {
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    client
        .read_property(mac, analog, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .map(drop)
}

async fn read(client: &Client, mac: &[u8]) {
    try_read(client, mac).await.unwrap();
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
    let (mut session, client, sends) = paced(50, 0, 0, Duration::ZERO).await;
    let start = Instant::now();
    for _ in 0..3 {
        read(&client, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 50, 100]));
    session.stop().await.unwrap();
}

/// The pause runs from the answer, as `BACnetClient`'s does: a device that
/// takes 30 ms to answer still gets the whole 50 ms before the next request.
#[tokio::test(start_paused = true)]
async fn an_endpoint_request_waits_the_interval_from_a_slow_answer() {
    let (mut session, client, sends) = paced(50, 0, 0, Duration::from_millis(30)).await;
    let start = Instant::now();
    for _ in 0..3 {
        read(&client, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 80, 160]));
    session.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn endpoint_requests_to_two_destinations_do_not_wait_on_each_other() {
    let (mut session, client, sends) = paced(50, 0, 0, Duration::ZERO).await;
    let start = Instant::now();
    tokio::join!(read(&client, A), read(&client, B));
    tokio::join!(read(&client, A), read(&client, B));
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 50]));
    assert_eq!(sent_to(&sends, B, start), ms(&[0, 50]));
    session.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_endpoint_interval_of_zero_sends_at_once() {
    let (mut session, client, sends) = paced(0, 0, 0, Duration::ZERO).await;
    let start = Instant::now();
    for _ in 0..3 {
        read(&client, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), [Duration::ZERO; 3]);
    session.stop().await.unwrap();
}

/// A retry goes when the APDU timeout runs out, not the interval: it keeps
/// the turn its request took. The next request waits the interval from the
/// answer to the retry.
#[tokio::test(start_paused = true)]
async fn an_endpoint_retry_is_not_held_back_by_the_interval() {
    let (mut session, client, sends) = paced(500, 1, 1, Duration::ZERO).await;
    let start = Instant::now();
    read(&client, A).await;
    read(&client, A).await;
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 100, 600]));
    session.stop().await.unwrap();
}

/// A caller that gives up on a request it sent finishes it then: the next
/// request waits the interval from that moment. Giving up while still
/// waiting leaves nothing behind.
#[tokio::test(start_paused = true)]
async fn an_endpoint_request_given_up_counts_as_finished_when_given_up() {
    let (mut session, client, sends) = paced(50, 0, 1, Duration::ZERO).await;
    let start = Instant::now();
    // Sent at 0, never answered, given up at 20.
    let abandoned = tokio::time::timeout(Duration::from_millis(20), try_read(&client, A)).await;
    assert!(abandoned.is_err());
    // Due at 70; given up while waiting, at 30.
    let waiting = tokio::time::timeout(Duration::from_millis(10), try_read(&client, A)).await;
    assert!(waiting.is_err());
    read(&client, A).await;
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 70]));
    session.stop().await.unwrap();
}

/// A request waiting for its turn when the session stops fails at once,
/// rather than at its turn minutes later, with the error a request the stop
/// cut off in flight gets (#2720).
#[tokio::test(start_paused = true)]
async fn a_waiting_endpoint_request_fails_at_once_when_the_session_stops() {
    let (mut session, client, sends) = paced(300_000, 0, 0, Duration::ZERO).await;
    read(&client, A).await;
    let waiter = Arc::clone(&client);
    let waiting = tokio::spawn(async move { try_read(&waiter, A).await });
    // Run until idle: the read is waiting for its turn.
    tokio::time::sleep(Duration::from_secs(1)).await;
    session.stop().await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(1), waiting).await;
    let error = ended.expect("ended at once").unwrap().unwrap_err();
    assert!(
        matches!(error, Error::RequestCutOff { sent: false }),
        "{error:?}"
    );
    assert_eq!(sends.lock().unwrap().len(), 1);
}

/// A request already sent when the session stops is cut off as one that may
/// have gone out (#1833): the device may have carried it out.
#[tokio::test(start_paused = true)]
async fn an_open_endpoint_request_is_cut_off_as_sent_when_the_session_stops() {
    let (mut session, client, sends) = paced(0, 0, 1, Duration::ZERO).await;
    let reader = Arc::clone(&client);
    let open = tokio::spawn(async move { try_read(&reader, A).await });
    // Well inside the 100 ms APDU timeout: the device stays silent.
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(sends.lock().unwrap().len(), 1, "the read went out");
    session.stop().await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(1), open).await;
    let error = ended.expect("ended at once").unwrap().unwrap_err();
    assert!(
        matches!(error, Error::RequestCutOff { sent: true }),
        "{error:?}"
    );
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
    assert!(matches!(
        new(hour + 1),
        Err(Error::InvalidConfig(text)) if text.to_string()
            == "endpoint session min-request-interval 3600001 ms is more than 3600000 ms"
    ));
}
