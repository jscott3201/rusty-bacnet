//! A COV-multiple context's Max_Notification_Delay backstop and confirmed
//! retries on an endpoint session (#2490), against `BACnetServer` at the
//! same virtual instants. Each host has a subscriber of its own, both are
//! driven alike, and every APDU each host sends is recorded with the time
//! it went. A refused report's change goes out again once the context's
//! Max_Notification_Delay has passed, and a subscriber that never answers
//! hears the same resends, hold-offs and held changes from both hosts over
//! the whole lifetime.

use super::*;
use bacnet_encoding::apdu::ErrorPdu;

/// What one host sent, each APDU with the milliseconds since the start it
/// went at: a confirmed request by its service parameters, since the two
/// hosts' invoke IDs differ, anything else whole.
type Heard = Vec<(u64, Bytes)>;

/// The two hosts of a clocked [`Pair`], each with a subscriber of its own.
struct Twins {
    pair: Pair,
    /// The endpoint's subscriber; the pair's own node is the server's.
    other: Node,
    start: tokio::time::Instant,
    /// What each host sent, the server's first.
    heard: [Heard; 2],
    /// The invoke ID of the last confirmed report each host sent.
    last: [Option<u8>; 2],
}

impl Twins {
    async fn new() -> Self {
        Self {
            pair: Pair::clocked().await,
            other: Node::new().await,
            start: tokio::time::Instant::now(),
            heard: [Vec::new(), Vec::new()],
            last: [None, None],
        }
    }

    /// The server's MAC, then the endpoint's.
    fn macs(&self) -> [MacAddr; 2] {
        [
            MacAddr::from_slice(self.pair.server.local_mac()),
            mac_of(&self.pair.endpoint),
        ]
    }

    /// Each subscriber sends `apdu(invoke_id)` to its host, `invoke_id`
    /// being the last report's of that host, or 1 when there is none yet.
    async fn send(&mut self, apdu: impl Fn(u8) -> Bytes) {
        let [server, endpoint] = self.macs();
        let ids = self.last.map(|id| id.unwrap_or(1));
        self.pair.node.send(&server, &apdu(ids[0])).await;
        self.other.send(&endpoint, &apdu(ids[1])).await;
    }

    /// AI-1 at `value` on both hosts, at one instant.
    async fn change(&mut self, value: f32) {
        let value = PropertyValue::Real(value);
        let server = &self.pair.server;
        server
            .set_present_value_local(&ai1(), value.clone())
            .await
            .unwrap();
        let endpoint = &self.pair.endpoint;
        endpoint
            .set_present_value_local(&ai1(), value)
            .await
            .unwrap();
    }

    /// Record what both hosts send until `at` past the start.
    async fn until(&mut self, at: Duration) {
        let deadline = self.start + at;
        loop {
            // No lock is taken in any arm.
            let (host, received) = tokio::select! {
                biased;
                Some(received) = self.pair.node.rx.recv() => (0, received),
                Some(received) = self.other.rx.recv() => (1, received),
                () = tokio::time::sleep_until(deadline) => return,
            };
            let went = (tokio::time::Instant::now() - self.start).as_millis();
            let apdu = decode_npdu(received.npdu).unwrap().payload;
            let heard = match decode_apdu(apdu.clone()) {
                Ok(Apdu::ConfirmedRequest(request)) => {
                    self.last[host] = Some(request.invoke_id);
                    request.service_request
                }
                _ => apdu,
            };
            self.heard[host].push((went as u64, heard));
        }
    }

    /// When each APDU went, after checking that both hosts sent the same
    /// at the same times.
    fn times(&self, what: &str) -> Vec<u64> {
        assert_eq!(
            self.heard[1], self.heard[0],
            "{what}: endpoint, then server"
        );
        self.heard[1].iter().map(|(at, _)| *at).collect()
    }
}

/// A confirmed, timestamped subscription to AI-1 for 600 s, its
/// Max_Notification_Delay `delay` seconds.
fn context(delay: u32) -> Bytes {
    let body = multiple_body(1, true, &[(ai1(), true)], Some((600, delay)));
    confirmed_apdu(1, MULTIPLE, &body)
}

/// An Error refusing the ConfirmedCOVNotificationMultiple `invoke_id`.
fn refused(invoke_id: u8) -> Bytes {
    let mut apdu = BytesMut::new();
    let error = Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice: NOTIFY,
        error_class: ErrorClass::SERVICES,
        error_code: ErrorCode::SERVICE_REQUEST_DENIED,
        error_data: Bytes::new(),
    });
    encode_apdu(&mut apdu, &error).unwrap();
    apdu.freeze()
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// The first report acknowledged, a change's report refused: the change
/// goes out again, with its history, once the Max_Notification_Delay has
/// passed, at the same instant from both hosts.
#[tokio::test(start_paused = true)]
async fn a_refused_change_goes_by_the_delay_as_on_the_server() {
    let mut twins = Twins::new().await;
    twins.send(|_| context(30)).await;
    twins.until(ms(100)).await;
    twins
        .send(|id| Bytes::copy_from_slice(&report_acked(id)))
        .await;
    twins.until(ms(200)).await;
    twins.change(25.0).await;
    twins.until(ms(300)).await;
    twins.send(refused).await;
    twins.until(ms(31_000)).await;
    // The reply and the first report, the change's report, and the
    // backstop's, 30 s after the change.
    assert_eq!(twins.times("refused"), [0, 0, 200, 30_200]);
    twins
        .send(|id| Bytes::copy_from_slice(&report_acked(id)))
        .await;
    twins.until(ms(120_000)).await;
    assert_eq!(twins.times("answered").len(), 4, "nothing more");
    twins.pair.stop().await;
}

/// A subscriber that never answers, the delay 10 s: each report goes again
/// at the timeout's 3 s, three times, then the context is held off for one
/// retry cycle, and a change made meanwhile goes when the hold-off ends. Both
/// hosts send at the same instants over the whole 600 s lifetime, and
/// nothing after it.
#[tokio::test(start_paused = true)]
async fn a_silent_subscriber_hears_the_same_cycle_over_the_lifetime() {
    let mut twins = Twins::new().await;
    twins.send(|_| context(DELAY)).await;
    twins.until(ms(14_000)).await;
    twins.change(25.0).await;
    twins.until(ms(40_000)).await;
    twins.change(26.0).await;
    twins.until(ms(700_000)).await;
    let times = twins.times("silent");
    let seconds: Vec<_> = times.iter().map(|at| at / 1000).collect();
    // The reply, then the first report and its resends; the change at 14 s
    // once the hold-off ends at 24 s; the one at 40 s at 48 s.
    assert_eq!(seconds[..10], [0, 0, 3, 6, 9, 24, 27, 30, 33, 48]);
    assert!(
        times.iter().all(|&at| at < 600_000),
        "nothing past the lifetime: {times:?}"
    );
    twins.pair.stop().await;
}
