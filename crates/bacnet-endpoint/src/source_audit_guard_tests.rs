//! An audited request waits for the database in a task the session owns,
//! not in its caller's (#2755): Tokio's lock hands a queued waiter the
//! guard whether or not anything polls it again, so a caller parked in a
//! `select!`, `join!` or `buffer_unordered` could own the node's database,
//! and a slot, until it was dropped. These callers are polled by hand, or
//! parked in a stream whose loop body awaits another read.
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use super::*;
use crate::session::eventually::eventually;
use crate::source_audit::slots::SLOTS;
use bacnet_types::primitives::BACnetTimeStamp;
use futures_util::StreamExt;
use futures_util::future::Either;

/// How long a test waits for what it expects, in paused time.
const WAIT: Duration = Duration::from_secs(5);

/// A read of the peer's analog input at `station`, as a caller holds it.
type Read<'a> = Pin<Box<dyn Future<Output = Result<ReadPropertyACK, Error>> + Send + 'a>>;

fn read_of<'a>(client: &'a BACnetClient<TestTransport>, station: &'a Station) -> Read<'a> {
    Box::pin(client.read_property(station, analog(), PropertyIdentifier::PRESENT_VALUE, None))
}

/// Poll `read` once, with a waker that wakes nothing: its caller then
/// stops polling.
fn poll_once(read: &mut Read<'_>) -> Poll<Result<ReadPropertyACK, Error>> {
    read.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

/// What the session sent, in turn: its confirmed requests and where each
/// went (sorted by link), its answers' invoke IDs, and its Audit records
/// for `SINK`.
#[derive(Default)]
struct Frames {
    requests: Vec<(Route, ConfirmedRequest)>,
    answers: Vec<u8>,
    records: Vec<BACnetAuditNotification>,
}

impl Frames {
    /// The invoke IDs the records name, sorted.
    fn recorded(&self) -> Vec<Option<u8>> {
        let mut ids: Vec<_> = self.records.iter().map(|record| record.invoke_id).collect();
        ids.sort_unstable();
        ids
    }
}

impl Endpoint {
    /// The session's client, the one [`Endpoint::read`] uses.
    fn the_client(&self) -> Arc<BACnetClient<TestTransport>> {
        Arc::clone(
            self.client
                .get_or_init(|| Arc::new(self.session.client().unwrap())),
        )
    }

    /// The next `count` frames the session sends.
    async fn frames(&mut self, count: usize) -> Frames {
        let mut frames = Frames::default();
        for _ in 0..count {
            let sent = self.next().await;
            let npdu = decode_npdu(sent.npdu.clone()).unwrap();
            let route = Route {
                link: sent.link.clone(),
                destination: npdu.destination,
            };
            match decode_apdu(npdu.payload).unwrap() {
                Apdu::ConfirmedRequest(request) => frames.requests.push((route, request)),
                Apdu::ComplexAck(ack) => {
                    assert_eq!(route, local(PEER), "an answer goes back to the peer");
                    frames.answers.push(ack.invoke_id);
                }
                _ => frames.records.push(record(sent, SINK)),
            }
        }
        frames
            .requests
            .sort_by_key(|(route, _)| route.link.to_vec());
        frames
    }

    /// The peer reads the node's Device's Object_Name under `invoke_id`: a
    /// request the node's server serves from the database.
    async fn served_read(&self, invoke_id: u8) {
        let mut service = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid(ObjectType::DEVICE, 123),
            property_identifier: PropertyIdentifier::OBJECT_NAME,
            property_array_index: None,
        }
        .encode(&mut service);
        let request = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: service.freeze(),
        });
        self.deliver_apdu(request, mac(PEER), None).await;
    }

    /// Answer `request` from the station on this link it went to.
    async fn answer(&self, (route, request): &(Route, ConfirmedRequest)) {
        self.deliver_apdu(ack(request, 5), route.link.clone(), None)
            .await;
    }
}

/// A caller whose audited read was admitted while the application held the
/// database, and that is never polled again, holds up nothing once the
/// guard comes free: a request the node serves is answered, another audited
/// read goes, and the parked read goes too. Both are recorded, and the
/// parked read, polled again, returns its answer.
#[tokio::test(start_paused = true)]
async fn a_caller_that_stops_polling_holds_no_database_guard() {
    a_parked_caller_holds_up_nothing().await;
}

/// [`a_caller_that_stops_polling_holds_no_database_guard`] with the
/// session's tasks on other threads than the test's, on real time: it
/// waits only for what must happen, each wait bounded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_caller_that_stops_polling_holds_no_database_guard_on_threads() {
    a_parked_caller_holds_up_nothing().await;
}

async fn a_parked_caller_holds_up_nothing() {
    let mut endpoint = source(SessionRole::Both, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let guard = db.write().await;
    let station = direct_read();
    let mut parked = read_of(&client, &station);
    assert!(poll_once(&mut parked).is_pending());
    assert_eq!(source.available_operations(), SLOTS - 1, "admitted");
    drop(guard);
    endpoint.served_read(7).await;
    let other = endpoint.read(Station::local(&mac(31)));
    let frames = endpoint.frames(3).await;
    assert_eq!(frames.answers, [7], "the node served the peer");
    assert!(frames.records.is_empty());
    let [to_peer, to_other]: [_; 2] = frames.requests.try_into().expect("both reads");
    assert_eq!((&to_peer.0, &to_other.0), (&local(PEER), &local(31)));
    endpoint.answer(&to_other).await;
    endpoint.answer(&to_peer).await;
    assert!(bounded(other).await.unwrap().is_ok());
    let mut ids = vec![Some(to_peer.1.invoke_id), Some(to_other.1.invoke_id)];
    ids.sort_unstable();
    assert_eq!(endpoint.frames(2).await.recorded(), ids, "both recorded");
    assert!(bounded(parked).await.is_ok());
    // The task gives the slot back once it has answered.
    bounded(async {
        while source.available_operations() != SLOTS {
            tokio::task::yield_now().await;
        }
    })
    .await;
    drop(client);
    endpoint.stop().await;
}

/// A loop that drains a `buffer_unordered` stream of audited reads and
/// reads again in its body, while a read the stream holds waits for the
/// database the application has: the body's read waits behind it, and
/// both go once the guard comes free, though nothing polls the stream.
#[tokio::test(start_paused = true)]
async fn a_buffered_read_waiting_for_the_database_holds_up_no_body_read() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let (go, gate) = tokio::sync::oneshot::channel::<()>();
    let (body, mut bodies) = mpsc::unbounded_channel();
    let reader = Arc::clone(&client);
    let drain = tokio::spawn(async move {
        let read = |last: u8| {
            let client = Arc::clone(&reader);
            async move {
                let station = Station::local(&mac(last));
                read_of(&client, &station).await
            }
        };
        let gated = read(31);
        let reads = [
            Either::Left(read(30)),
            Either::Right(async move {
                gate.await.unwrap();
                gated.await
            }),
        ];
        let mut reads = futures_util::stream::iter(reads).buffer_unordered(2);
        let mut answers = 0;
        while let Some(answer) = reads.next().await {
            answer.unwrap();
            body.send(()).unwrap();
            read(32).await.unwrap();
            answers += 2;
        }
        answers
    });
    let first = endpoint.request().await;
    assert_eq!(first.0, local(30));
    let guard = db.write().await;
    go.send(()).unwrap();
    eventually(WAIT, "the gated read holds a slot", async || {
        source.available_operations() == SLOTS - 2
    })
    .await;
    // The first ends, and the body reads 32 behind the gated read's wait.
    endpoint.answer(&first).await;
    let frames = endpoint.frames(1).await;
    assert_eq!(frames.recorded(), [Some(first.1.invoke_id)]);
    bounded(bodies.recv()).await.unwrap();
    assert_eq!(source.available_operations(), SLOTS - 2, "gated and body");
    drop(guard);
    let frames = endpoint.frames(2).await;
    let [gated, again]: [_; 2] = frames.requests.try_into().expect("gated and body");
    assert_eq!((&gated.0, &again.0), (&local(31), &local(32)));
    endpoint.answer(&again).await;
    endpoint.answer(&gated).await;
    // Both recorded, and the body reads 32 once more for the gated read.
    let frames = endpoint.frames(3).await;
    assert_eq!(frames.records.len(), 2);
    let [last]: [_; 1] = frames.requests.try_into().expect("the body's last read");
    endpoint.answer(&last).await;
    assert_eq!(endpoint.frames(1).await.records.len(), 1);
    assert_eq!(bounded(drain).await.unwrap(), 4);
    drop(client);
    endpoint.stop().await;
}

/// A caller that goes away while its admitted read waits for the database
/// gets its slot back at once, though the guard is still held, and one
/// that goes as the guard comes free sends nothing: neither sends, records
/// or takes a sequence number, and no guard is left held.
#[tokio::test(start_paused = true)]
async fn a_caller_gone_while_its_read_waits_for_the_database_sends_nothing() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let station = direct_read();
    let guard = db.write().await;
    let mut parked = read_of(&client, &station);
    assert!(poll_once(&mut parked).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    drop(parked);
    eventually(WAIT, "the slot is back, the guard still held", async || {
        source.available_operations() == SLOTS
    })
    .await;
    let mut parked = read_of(&client, &station);
    assert!(poll_once(&mut parked).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    // The guard goes to the read's wait, and its caller goes before that
    // wait is polled again.
    drop(guard);
    drop(parked);
    eventually(WAIT, "that slot is back too", async || {
        source.available_operations() == SLOTS
    })
    .await;
    assert!(db.try_write().is_ok(), "no guard is held");
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(endpoint.sent.try_recv().is_err(), "nothing was sent");
    assert_eq!(endpoint.session.coordinator.active_count().unwrap(), 0);
    let audit = audited_read(&mut endpoint, direct_read(), SINK).await;
    assert_eq!(
        audit.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0)),
        "the next read's record takes the first number"
    );
    drop(client);
    endpoint.stop().await;
}

/// A stop while the read of a caller that is never polled again waits for
/// the database the application holds: the stop waits for the guard itself
/// (to take its Reporter out), gets it once the application lets go, not
/// after the read, and the read ends cut off, unsent and unrecorded: its
/// task, which the stop aborted, can't say it hadn't sent the read, so it
/// is reported as one that may have been (#1833).
#[tokio::test(start_paused = true)]
async fn a_stop_ends_a_read_whose_caller_stopped_polling() {
    let mut endpoint = source(SessionRole::Both, false).await;
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let station = direct_read();
    let guard = db.write().await;
    let mut parked = read_of(&client, &station);
    assert!(poll_once(&mut parked).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    let stop = endpoint.session.stop();
    tokio::pin!(stop);
    assert!(timeout(Duration::from_millis(10), &mut stop).await.is_err());
    drop(guard);
    bounded(stop).await.unwrap();
    assert!(matches!(
        bounded(parked).await,
        Err(Error::RequestCutOff { sent: true })
    ));
    assert!(endpoint.sent.try_recv().is_err(), "nothing was sent");
}

/// A read the Reporter doesn't select runs in its caller's task, as an
/// unaudited request does, whether its caller planned it (the database
/// free) or its task did and handed it back (the database busy): dropping
/// the caller once it is sent ends its transaction, with no answer and no
/// record.
#[tokio::test(start_paused = true)]
async fn a_read_the_reporter_does_not_select_stays_its_callers() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    db.write()
        .await
        .get_mut(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::NONE,
            AuditOperationFlags::from_bits(1).unwrap(),
            false,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
    for busy in [false, true] {
        let guard = busy.then(|| db.try_write().unwrap());
        let read = endpoint.read(direct_read());
        tokio::time::sleep(Duration::from_millis(1)).await;
        drop(guard);
        let (route, _) = endpoint.request().await;
        assert_eq!(route, local(PEER));
        assert_eq!(endpoint.session.coordinator.active_count().unwrap(), 1);
        read.abort();
        let _ = read.await;
        eventually(WAIT, "the transaction ended with its caller", async || {
            endpoint.session.coordinator.active_count().unwrap() == 0
        })
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            endpoint.sent.try_recv().is_err(),
            "busy {busy}: no record or retry"
        );
    }
    endpoint.stop().await;
}

/// An I-Am polled once while the application holds the node's database,
/// then never again, holds up nothing once the guard comes free: its
/// Device check runs in a task of the session's, so a writer queued behind
/// it gets the database. Polled at last, the I-Am goes (#2772).
#[tokio::test(start_paused = true)]
async fn a_parked_i_am_holds_up_no_database_writer() {
    let mut endpoint = source(SessionRole::Both, false).await;
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let guard = db.write().await;
    let mut parked = Box::pin(endpoint.session.broadcast_i_am());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(parked.as_mut().poll(&mut cx).is_pending());
    let writer = tokio::spawn({
        let db = Arc::clone(&db);
        async move { drop(db.write().await) }
    });
    tokio::task::yield_now().await;
    drop(guard);
    bounded(writer).await.unwrap();
    assert!(bounded(parked).await.is_ok());
    // The I-Am is among what the session sent; `next` fails the test if it
    // never comes.
    loop {
        let npdu = decode_npdu(endpoint.next().await.npdu).unwrap();
        if let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap()
            && request.service_choice == bacnet_types::enums::UnconfirmedServiceChoice::I_AM
        {
            break;
        }
    }
    endpoint.stop().await;
}

/// The I-Am's future is still `Send` and `Sync`, as the server's local
/// calls' are, though it may now hold a boxed task across an await (#2781):
/// checked when this compiles.
#[allow(dead_code)]
fn the_i_am_is_send_and_sync<T: TransportPort + 'static>(session: &EndpointSession<T>) {
    fn check(_: impl Send + Sync) {}
    check(session.broadcast_i_am());
}

#[path = "source_audit_plan_tests.rs"]
mod plan;

#[path = "source_recipient_guard_tests.rs"]
mod recipient_guard;
