//! A foreign device's registration schedule on the node engine
//! (`Retry::Backoff`, #1622). After a success the next attempt goes one
//! renewal interval into the lease it proved, which starts at the earliest
//! send the success could answer; after a refusal, one interval after that
//! attempt started; after a loss, sooner: one response timeout after it,
//! doubling with each loss in a row, and never later than the interval. A
//! Distribute-Broadcast-To-Network NAK from the BBMD pulls the next attempt
//! forward. Each attempt waits for its result for the interval or 3 s,
//! whichever is shorter.

use std::time::Duration;

use bacnet_bip_core::node::Mode;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::send_sink::SendSink;
use super::*;

const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 0xBAC0);
const BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 0xBAC0);
pub(super) const BBMD: ([u8; 4], u16) = ([198, 51, 100, 1], 0xBAC0);

/// A foreign device renewing every `interval`, what it sends, what it hands
/// up, and its driver.
pub(super) struct Device {
    pub(super) ctx: RecvContext,
    pub(super) sink: SendSink,
    pub(super) npdus: mpsc::Receiver<ReceivedNpdu>,
    start: Instant,
    _driver: JoinHandle<()>,
}

impl Device {
    pub(super) async fn new(interval: Duration) -> Self {
        let sink = SendSink::default();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
        let config = ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::from(BBMD.0),
            bbmd_port: BBMD.1,
            ttl: 60,
            renewal_interval: Some(interval),
        };
        let mode = Mode::Foreign(config.engine(config.interval().unwrap()));
        let link = test_support::test_link(OWN, BROADCAST, mode);
        let (npdu_tx, npdus) = mpsc::channel(4);
        let ctx = RecvContext::on_link(socket, npdu_tx, link, None);
        let driver = tokio::spawn(Arc::clone(&ctx.node).drive());
        Self {
            ctx,
            sink,
            npdus,
            start: Instant::now(),
            _driver: driver,
        }
    }

    /// Let time run to `ms` milliseconds from the start.
    pub(super) async fn at(&self, ms: u64) {
        tokio::time::sleep_until(self.start + Duration::from_millis(ms)).await;
    }

    /// `peer` sends a BVLC-Result with `code`.
    async fn says(&self, peer: ([u8; 4], u16), code: u16) {
        let result = BvllMessage {
            function: BvlcFunction::BVLC_RESULT,
            payload: bytes::Bytes::copy_from_slice(&code.to_be_bytes()),
        };
        handle_bvll_message(&result, peer, Delivery::Unicast, &self.ctx).await;
    }

    /// `msg` arrives from `sender`, sent as `delivery` says.
    pub(super) async fn hears(
        &self,
        msg: &BvllMessage,
        sender: ([u8; 4], u16),
        delivery: Delivery,
    ) {
        handle_bvll_message(msg, sender, delivery, &self.ctx).await;
    }

    /// The BBMD answers with `code`.
    pub(super) async fn bbmd_says(&self, code: u16) {
        self.says(BBMD, code).await;
    }

    /// What went to the BBMD since the last look, with when it went, in
    /// milliseconds from the start.
    pub(super) fn sent(&self) -> Vec<(u128, Vec<u8>)> {
        let bbmd = SocketAddrV4::new(Ipv4Addr::from(BBMD.0), BBMD.1);
        self.sink
            .take()
            .into_iter()
            .filter(|sent| sent.to == bbmd)
            .map(|sent| ((sent.at - self.start).as_millis(), sent.bytes))
            .collect()
    }

    /// When each registration attempt went, in milliseconds from the start.
    pub(super) fn attempts(&self) -> Vec<u128> {
        self.sent()
            .into_iter()
            .filter(|(_, bytes)| *bytes == [0x81, 0x05, 0x00, 0x06, 0x00, 60])
            .map(|(at, _)| at)
            .collect()
    }
}

#[tokio::test(start_paused = true)]
async fn attempts_back_off_after_a_loss_and_renew_into_the_lease() {
    let device = Device::new(Duration::from_secs(10)).await;
    // The first goes at once, and is answered: registered until 60 s.
    device.at(13).await;
    device.bbmd_says(0).await;
    // The second is refused: the next waits one interval.
    device.at(10_013).await;
    device.bbmd_says(0x30).await;
    // The third and fourth are lost: the retry after each comes one
    // response timeout (3 s), then two, after it timed out.
    device.at(29_000).await;
    tokio::task::yield_now().await;
    let lost = device.ctx.node.snapshot();
    let registration = lost.foreign_registration;
    assert_eq!(lost.register_foreign_device.timeouts, 2);
    assert_eq!(
        registration.last_outcome,
        Some(ForeignRegistrationOutcome::TimedOut)
    );
    assert_eq!(registration.next_attempt_in, Some(Duration::from_secs(6)));
    // The first lease still stands.
    assert!(registration.registered);
    assert_eq!(registration.lease_remaining, Some(Duration::from_secs(31)));
    // The fifth is answered. The success could answer any attempt since the
    // last answer, so the lease starts at the third's send, 20 s, and its
    // renewal, due at 30 s, goes at once.
    device.at(35_013).await;
    device.bbmd_says(0).await;
    tokio::task::yield_now().await;
    // That one's answer starts the lease at the fifth's send.
    device.at(35_026).await;
    device.bbmd_says(0).await;
    device.at(40_000).await;

    assert_eq!(
        device.attempts(),
        [0, 10_000, 20_000, 26_000, 35_000, 35_013]
    );
    let snapshot = device.ctx.node.snapshot();
    let counters = snapshot.register_foreign_device;
    assert_eq!(
        (counters.sent, counters.results, counters.timeouts),
        (6, 4, 2)
    );
    let registration = snapshot.foreign_registration;
    assert_eq!(
        (
            registration.attempts,
            registration.accepted,
            registration.refused
        ),
        (6, 3, 1)
    );
    assert_eq!(registration.next_attempt_in, Some(Duration::from_secs(5)));
    assert!(registration.registered);
    assert_eq!(registration.lease_remaining, Some(Duration::from_secs(55)));
    assert_eq!(registration.lapses, 0);
}

/// A Distribute-Broadcast-To-Network NAK from the BBMD says it holds no
/// registration for this device (Annex J.4.5): the next attempt goes as
/// soon as one response timeout has passed since the last one started. The
/// same NAK from another peer changes nothing.
#[tokio::test(start_paused = true)]
async fn a_distribute_nak_from_the_bbmd_pulls_the_next_attempt_forward() {
    let device = Device::new(Duration::from_secs(10)).await;
    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(10_013).await;
    device.bbmd_says(0).await;
    // Renewed from 10 s; the next is due at 20 s.
    device.at(12_000).await;
    device.says(([192, 0, 2, 30], 0xBAC0), 0x60).await;
    assert_eq!(
        device
            .ctx
            .node
            .snapshot()
            .foreign_registration
            .next_attempt_in,
        Some(Duration::from_secs(8))
    );
    device.bbmd_says(0x60).await;
    device.at(13_500).await;

    assert_eq!(device.attempts(), [0, 10_000, 13_000]);
    let snapshot = device.ctx.node.snapshot();
    assert_eq!(snapshot.foreign_registration.renewal_hints, 1);
    assert_eq!(snapshot.unmatched_responses, 2);
}

/// An attempt waits for its result for the interval when that is shorter
/// than 3 s, so a lost one ends as the next falls due. An answer to the
/// second could also answer the first, still unanswered, so the lease starts
/// at the first's send, and its renewal is due at once.
#[tokio::test(start_paused = true)]
async fn a_short_interval_bounds_the_wait_and_a_late_answer_anchors_the_lease_early() {
    let device = Device::new(Duration::from_millis(300)).await;
    device.at(303).await;
    device.bbmd_says(0).await;
    device.at(950).await;
    assert_eq!(device.attempts(), [0, 300, 303, 603, 903]);
    let snapshot = device.ctx.node.snapshot();
    assert_eq!(snapshot.register_foreign_device.timeouts, 3);
    assert_eq!(snapshot.foreign_registration.accepted, 1);
}
