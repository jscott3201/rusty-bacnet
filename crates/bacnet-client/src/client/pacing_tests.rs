//! The minimum interval between confirmed requests to one destination,
//! on paused time (#1535).
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::apdu::{self, encode_apdu, Apdu, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::port::{ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

use super::pacing::{with_capacity, PaceKey, RequestPacer};
use super::{BACnetClient, ConfirmedTarget};

type Sends = Arc<StdMutex<Vec<(Instant, MacAddr)>>>;

const A: &[u8] = &[0x0A];
const B: &[u8] = &[0x0B];
const C: &[u8] = &[0x0C];

/// Records when each unicast leaves and answers a confirmed request with a
/// SimpleAck from the MAC it went to, `reply_after` later; the first
/// `unanswered` requests get no answer.
struct RecordingTransport {
    local_mac: MacAddr,
    inbound_tx: mpsc::Sender<ReceivedNpdu>,
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    sends: Sends,
    reply_after: Duration,
    unanswered: StdMutex<usize>,
}

impl TransportPort for RecordingTransport {
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
        let npdu = decode_npdu(Bytes::copy_from_slice(npdu))?;
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload)? else {
            return Ok(());
        };
        {
            let mut unanswered = self.unanswered.lock().unwrap();
            if *unanswered > 0 {
                *unanswered -= 1;
                return Ok(());
            }
        }
        let mut apdu_buf = BytesMut::new();
        encode_apdu(
            &mut apdu_buf,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            }),
        )?;
        let mut npdu_buf = BytesMut::new();
        encode_npdu(
            &mut npdu_buf,
            &Npdu {
                payload: apdu_buf.freeze(),
                ..Npdu::default()
            },
        )?;
        let received = ReceivedNpdu {
            direct_response: None,
            npdu: npdu_buf.freeze(),
            source_mac: MacAddr::from_slice(mac),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        };
        let inbound = self.inbound_tx.clone();
        let delay = self.reply_after;
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = inbound.send(received).await;
        });
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

async fn paced_client(
    interval_ms: u64,
    reply_after: Duration,
    unanswered: usize,
) -> (BACnetClient<RecordingTransport>, Sends) {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    let sends = Sends::default();
    let transport = RecordingTransport {
        local_mac: MacAddr::from_slice(&[0x01]),
        inbound_tx,
        inbound_rx: Some(inbound_rx),
        sends: Arc::clone(&sends),
        reply_after,
        unanswered: StdMutex::new(unanswered),
    };
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_timeout_ms(10_000)
        .apdu_retries(0)
        .min_request_interval_ms(interval_ms)
        .build()
        .await
        .unwrap();
    (client, sends)
}

async fn client(interval_ms: u64) -> (BACnetClient<RecordingTransport>, Sends) {
    paced_client(interval_ms, Duration::ZERO, 0).await
}

async fn request(client: &BACnetClient<RecordingTransport>, mac: &[u8]) {
    client
        .confirmed_request(mac, ConfirmedServiceChoice::WRITE_PROPERTY, &[0x0C])
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
async fn requests_to_one_destination_are_spaced_by_the_interval() {
    let (mut client, sends) = client(50).await;
    let start = Instant::now();
    for _ in 0..3 {
        request(&client, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 50, 100]));
    client.stop().await.unwrap();
}

/// The pause runs from the reply, not from the send: a device that takes
/// 30 ms to answer still gets the whole 50 ms before the next request.
#[tokio::test(start_paused = true)]
async fn a_slow_reply_then_the_next_request_waits_the_full_pause() {
    let (mut client, sends) = paced_client(50, Duration::from_millis(30), 0).await;
    let start = Instant::now();
    for _ in 0..3 {
        request(&client, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 80, 160]));
    client.stop().await.unwrap();
}

/// While a request is outstanding, the next to the same destination waits
/// the interval from its send.
#[tokio::test(start_paused = true)]
async fn concurrent_requests_to_one_destination_are_spaced_from_the_send() {
    let (mut client, sends) = paced_client(50, Duration::from_millis(100), 0).await;
    let start = Instant::now();
    tokio::join!(
        request(&client, A),
        request(&client, A),
        request(&client, A)
    );
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 50, 100]));
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn requests_to_two_destinations_do_not_wait_on_each_other() {
    let (mut client, sends) = client(50).await;
    let start = Instant::now();
    tokio::join!(request(&client, A), request(&client, B));
    tokio::join!(request(&client, A), request(&client, B));
    let expected = ms(&[0, 50]);
    assert_eq!(sent_to(&sends, A, start), expected);
    assert_eq!(sent_to(&sends, B, start), expected);
    client.stop().await.unwrap();
}

/// A caller that gives up on a request it sent finishes it then: the next
/// request waits the interval from that moment. Giving up while still
/// waiting leaves nothing behind.
#[tokio::test(start_paused = true)]
async fn a_cancelled_request_counts_as_finished_when_cancelled() {
    let (mut client, sends) = paced_client(50, Duration::ZERO, 1).await;
    let start = Instant::now();
    // Sent at 0, never answered, abandoned at 20.
    let abandoned = tokio::time::timeout(
        Duration::from_millis(20),
        client.confirmed_request(A, ConfirmedServiceChoice::WRITE_PROPERTY, &[0x0C]),
    )
    .await;
    assert!(abandoned.is_err());
    // Due at 70; given up while waiting, at 30.
    let waiting = tokio::time::timeout(
        Duration::from_millis(10),
        client.confirmed_request(A, ConfirmedServiceChoice::WRITE_PROPERTY, &[0x0C]),
    )
    .await;
    assert!(waiting.is_err());
    request(&client, A).await;
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 70]));
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_interval_of_zero_sends_at_once() {
    let (mut client, sends) = client(0).await;
    let start = Instant::now();
    for _ in 0..3 {
        request(&client, A).await;
    }
    assert_eq!(sent_to(&sends, A, start), [Duration::ZERO; 3]);
    assert_eq!(client.pacer.remembered(), 0);
    client.stop().await.unwrap();
}

fn local(mac: &[u8]) -> PaceKey {
    PaceKey::of(ConfirmedTarget::Local { mac })
}

const I: Duration = Duration::from_millis(50);

/// Wait for a turn to A and record when it came; hold the request open
/// until `held_for` has passed.
async fn send_after(
    pacer: &Arc<RequestPacer>,
    sent: &StdMutex<Vec<Duration>>,
    start: Instant,
    held_for: Duration,
) {
    let _guard = pacer.wait(local(A)).await;
    sent.lock().unwrap().push(Instant::now() - start);
    tokio::time::sleep(held_for).await;
}

/// Every pair of sends at least `I` apart.
fn assert_spaced(sent: &StdMutex<Vec<Duration>>) {
    let mut sent = sent.lock().unwrap().clone();
    sent.sort();
    for pair in sent.windows(2) {
        assert!(pair[1] - pair[0] >= I, "{sent:?}");
    }
}

/// The review's repro: with one request out and two waiting, a runtime that
/// falls behind by several intervals must still not let the two go back to
/// back.
#[tokio::test(start_paused = true)]
async fn waiters_that_wake_late_together_still_go_an_interval_apart() {
    let pacer = Arc::new(RequestPacer::new(I));
    let start = Instant::now();
    let sent = StdMutex::new(vec![Duration::ZERO]);
    let _out = pacer.wait(local(A)).await;
    tokio::join!(
        send_after(&pacer, &sent, start, Duration::ZERO),
        send_after(&pacer, &sent, start, Duration::ZERO),
        tokio::time::advance(3 * I),
    );
    assert_eq!(sent.lock().unwrap().len(), 3);
    assert_spaced(&sent);
}

/// Waiters released together go one at a time, the interval apart.
#[tokio::test(start_paused = true)]
async fn waiters_released_together_go_an_interval_apart() {
    let pacer = Arc::new(RequestPacer::new(I));
    let start = Instant::now();
    let sent = StdMutex::new(Vec::new());
    let first = pacer.wait(local(A)).await;
    let release = async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        drop(first);
    };
    // Each holds its request open for a millisecond.
    let held = Duration::from_millis(1);
    tokio::join!(
        release,
        send_after(&pacer, &sent, start, held),
        send_after(&pacer, &sent, start, held),
        send_after(&pacer, &sent, start, held),
        send_after(&pacer, &sent, start, held),
    );
    let mut sent = sent.into_inner().unwrap();
    sent.sort();
    // The first goes once the request it waited on has been finished an
    // interval; each after that an interval after the one before finished.
    assert_eq!(sent, ms(&[60, 111, 162, 213]));
}

/// A waiter given up leaves the destination as it was.
#[tokio::test(start_paused = true)]
async fn a_waiter_given_up_leaves_nothing_behind() {
    let pacer = Arc::new(RequestPacer::new(I));
    let _out = pacer.wait(local(A)).await;
    let before = pacer.latest(&local(A));
    let given_up = tokio::time::timeout(Duration::from_millis(10), pacer.wait(local(A))).await;
    assert!(given_up.is_err());
    assert_eq!(pacer.latest(&local(A)), before);
    assert_eq!(pacer.remembered(), 1);
}

/// Only the latest request's finish counts: an earlier one finishing late
/// doesn't push the next request back.
#[tokio::test(start_paused = true)]
async fn an_earlier_finish_is_ignored() {
    let pacer = Arc::new(RequestPacer::new(I));
    let start = Instant::now();
    let a = pacer.wait(local(A)).await; // out from 0
    let b = pacer.wait(local(A)).await; // goes at 50, stays out
    assert_eq!(Instant::now() - start, I);
    let (c, ()) = tokio::join!(
        async {
            let _c = pacer.wait(local(A)).await; // due at 100
            Instant::now() - start
        },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(a); // at 70
        },
    );
    assert_eq!(c, Duration::from_millis(100));
    drop(b);
}

/// The same on the wire: a request made while the one before it is out
/// goes the whole interval after its reply.
#[tokio::test(start_paused = true)]
async fn a_concurrent_request_waits_the_full_pause_after_a_slow_reply() {
    let (mut client, sends) = paced_client(50, Duration::from_millis(40), 0).await;
    let start = Instant::now();
    tokio::join!(request(&client, A), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        request(&client, A).await;
    });
    assert_eq!(sent_to(&sends, A, start), ms(&[0, 90]));
    client.stop().await.unwrap();
}

/// At the cap an idle destination goes before one with a request out, even
/// one free sooner.
#[tokio::test(start_paused = true)]
async fn at_the_cap_an_idle_destination_goes_before_a_busy_one() {
    let pacer = with_capacity(Duration::from_millis(100), 2);
    let _a = pacer.wait(local(A)).await; // out: free at 100
    tokio::time::advance(Duration::from_millis(10)).await;
    drop(pacer.wait(local(B)).await); // idle: free at 110
    let _c = pacer.wait(local(C)).await;
    assert!(pacer.latest(&local(A)).is_some() && pacer.latest(&local(C)).is_some());
    assert!(pacer.latest(&local(B)).is_none());
}

/// A guard from a destination forgotten at the cap and made again changes
/// nothing in the new one.
#[tokio::test(start_paused = true)]
async fn a_stale_guard_leaves_a_destination_made_again_alone() {
    let pacer = with_capacity(I, 1);
    let stale = pacer.wait(local(A)).await; // out
    drop(pacer.wait(local(B)).await); // forgets A
    assert!(pacer.latest(&local(A)).is_none());
    let _again = pacer.wait(local(A)).await; // forgets B; A made again
    let before = pacer.latest(&local(A));
    assert_eq!(before.map(|(_, finished)| finished), Some(None));
    drop(stale);
    assert_eq!(pacer.latest(&local(A)), before);
}

#[tokio::test(start_paused = true)]
async fn the_pacer_forgets_idle_destinations_and_stays_bounded() {
    let pacer = Arc::new(RequestPacer::new(Duration::from_millis(10)));
    // Routed and local destinations with the same MAC are different devices.
    let routed = PaceKey::of(ConfirmedTarget::Routed {
        router_mac: B,
        dest_network: 5,
        dest_mac: A,
    });
    assert_ne!(routed, local(A));

    let many = |n: u32| {
        (0..n).map(|i| {
            PaceKey::of(ConfirmedTarget::Routed {
                router_mac: B,
                dest_network: 1,
                dest_mac: &i.to_be_bytes(),
            })
        })
    };
    // Every destination still inside its interval: the cap holds anyway.
    for key in many(5_000) {
        drop(pacer.wait(key).await);
    }
    assert!(pacer.remembered() <= 4_096);
    // Once they have all been idle a whole interval, a new one sweeps them.
    tokio::time::advance(Duration::from_millis(10)).await;
    drop(pacer.wait(routed).await);
    assert_eq!(pacer.remembered(), 1);
}

#[tokio::test]
async fn an_interval_over_an_hour_is_refused() {
    let (inbound_tx, inbound_rx) = mpsc::channel(1);
    let transport = RecordingTransport {
        local_mac: MacAddr::from_slice(&[0x01]),
        inbound_tx,
        inbound_rx: Some(inbound_rx),
        sends: Sends::default(),
        reply_after: Duration::ZERO,
        unanswered: StdMutex::new(0),
    };
    let refused = BACnetClient::generic_builder()
        .transport(transport)
        .min_request_interval_ms(super::MAX_MIN_REQUEST_INTERVAL_MS + 1)
        .build()
        .await;
    match refused {
        Err(Error::Encoding(message)) => assert!(message.contains("min-request-interval")),
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("an interval over an hour must be refused"),
    }
    let (mut client, _) = client(super::MAX_MIN_REQUEST_INTERVAL_MS).await;
    client.stop().await.unwrap();
}
