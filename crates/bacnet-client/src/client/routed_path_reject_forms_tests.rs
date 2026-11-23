//! How a reject reaches the client, and a busy router (#2724). A reject
//! sent as a broadcast, a global one or one on this link, is no one
//! request's, for reason 4 as for the others. One relayed from a farther
//! router names that router. A busy reject leaves the request to its
//! retries; if they run out, the timeout names the router, and a busy
//! reject for another request, network or station names nothing.

use bacnet_encoding::npdu::{NpduAddress, encode_npdu};
use bacnet_types::enums::AbortReason;
use bacnet_types::error::RemoteError;
use bytes::BytesMut;

use super::*;

/// How a reject reaches the client from the path's router.
#[derive(Clone, Copy, Debug)]
enum Form {
    /// As a global broadcast: DNET 65535, a broadcast on this link.
    Global,
    /// As a broadcast on this link, with no DNET.
    LinkBroadcast,
    /// To this station, from the router farther along at this network and
    /// MAC, which the path's router relayed.
    Relayed(u16, [u8; 1]),
}

impl Rig {
    async fn reject_as(&self, form: Form, reason: RejectMessageReason) {
        let [high, low] = DNET.to_be_bytes();
        let (destination, source, group) = match form {
            Form::Global => {
                let everywhere = NpduAddress {
                    network: 0xFFFF,
                    mac_address: MacAddr::new(),
                };
                (Some(everywhere), None, true)
            }
            Form::LinkBroadcast => (None, None, true),
            Form::Relayed(network, mac) => {
                let far = NpduAddress {
                    network,
                    mac_address: MacAddr::from_slice(&mac),
                };
                (None, Some(far), false)
            }
        };
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK),
                destination,
                source,
                hop_count: 255,
                payload: Bytes::copy_from_slice(&[reason.to_raw(), high, low]),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.inbound
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&ROUTER),
                link_layer_group: group,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
    }

    /// A request runs on past whatever came, retries one timeout after its
    /// first frame, and takes the device's answer.
    async fn runs_on(&mut self, task: JoinHandle<Result<Bytes, Error>>, first: u8, sent: Instant) {
        let (_, retry) = self.next().await;
        assert_eq!(sent.elapsed(), TIMEOUT);
        assert_eq!(retry.invoke_id, first);
        self.ack(retry.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
    }

    /// A request sent now gets no answer: its first frame and each of its
    /// three retries go, and it fails as the client's timeout fails one.
    async fn unanswered(&mut self, before_retries: impl AsyncFnOnce(&Self)) -> Error {
        let task = self.routed(1);
        self.next().await;
        before_retries(self).await;
        for _ in 0..3 {
            self.next().await;
        }
        task.await.unwrap().unwrap_err()
    }
}

fn plain_timeout(error: &Error) -> bool {
    matches!(error, Error::Remote(RemoteError::Abort { reason })
        if *reason == AbortReason::TSM_TIMEOUT)
}

/// A reject sent as a global broadcast, or as a broadcast on this link, is
/// for no one station, whatever its reason: the request runs on. For
/// reason 4 the request is one frame larger than the 228-octet floor, so
/// only the form keeps it from ending the request.
#[tokio::test(start_paused = true)]
async fn a_reject_sent_as_a_broadcast_ends_no_request() {
    for reason in [
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        RejectMessageReason::MESSAGE_TOO_LONG,
    ] {
        for form in [Form::Global, Form::LinkBroadcast] {
            let mut rig = Rig::new().await;
            rig.client
                .configure_routed_path_max_npdu(&ROUTER, DNET, 400)
                .unwrap();
            let task = rig.routed(300);
            let (_, first) = rig.next().await;
            assert!(!first.segmented);
            let sent = Instant::now();
            rig.reject_as(form, reason).await;
            rig.runs_on(task, first.invoke_id, sent).await;
            rig.stop().await;
        }
    }
}

/// A reject a farther router sent, relayed by the path's router, ends the
/// request and names both routers.
#[tokio::test(start_paused = true)]
async fn a_relayed_reject_names_the_router_that_sent_it() {
    let mut rig = Rig::new().await;
    let task = rig.routed(1);
    rig.next().await;
    rig.reject_as(
        Form::Relayed(5, [0x0A]),
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
    )
    .await;
    let reject = rejected(task.await.unwrap());
    assert_eq!(reject.router.as_slice(), ROUTER);
    assert_eq!(
        reject.refused_by,
        Some((5, bacnet_npdu::Mac::from_array([0x0A])))
    );
    assert!(
        reject
            .to_string()
            .starts_with("Reject-Message-To-Network through router mstp:2, from 5:0x0A: "),
        "{reject}"
    );
    rig.stop().await;
}

/// A busy router, then silence: the request takes its retries at the
/// normal interval and times out, and the timeout names the router and the
/// network.
#[tokio::test(start_paused = true)]
async fn a_busy_router_then_silence_is_a_timeout_naming_it() {
    let mut rig = Rig::new().await;
    let started = Instant::now();
    let error = rig
        .unanswered(async |rig: &Rig| {
            rig.reject(&ROUTER, RejectMessageReason::ROUTER_BUSY, DNET)
                .await;
        })
        .await;
    assert_eq!(started.elapsed(), 4 * TIMEOUT);
    let Error::RouterBusyTimeout(busy) = error else {
        panic!("expected a timeout naming the busy router, got {error:?}")
    };
    assert_eq!(busy.router.as_slice(), ROUTER);
    assert_eq!(busy.link, Some(DataLink::Mstp));
    assert_eq!(
        (busy.network, busy.reason),
        (DNET, RejectMessageReason::ROUTER_BUSY)
    );
    assert!(
        Error::RouterBusyTimeout(busy.clone())
            .to_string()
            .contains("router mstp:2: busy for network 100"),
    );
    rig.settled();
    rig.stop().await;
}

/// A busy router, then an answer on the retry: the request succeeds, and
/// the busy reject is gone with it, so the next request's timeout names no
/// router.
#[tokio::test(start_paused = true)]
async fn a_busy_router_then_an_answer_leaves_nothing_behind() {
    let mut rig = Rig::new().await;
    let task = rig.routed(1);
    let (_, first) = rig.next().await;
    let sent = Instant::now();
    rig.reject(&ROUTER, RejectMessageReason::ROUTER_BUSY, DNET)
        .await;
    rig.runs_on(task, first.invoke_id, sent).await;
    let error = rig.unanswered(async |_: &Rig| {}).await;
    assert!(plain_timeout(&error), "{error:?}");
    rig.stop().await;
}

/// A busy reject from another station, for another network, sent as a
/// broadcast, or decoded before the request went out is not the request's:
/// its timeout names no router.
#[tokio::test(start_paused = true)]
async fn a_busy_reject_for_another_request_or_path_is_not_kept() {
    let mut rig = Rig::new().await;
    // Before any request: the path has none to tie it to.
    rig.reject(&ROUTER, RejectMessageReason::ROUTER_BUSY, DNET)
        .await;
    let error = rig
        .unanswered(async |rig: &Rig| {
            rig.reject(&[9], RejectMessageReason::ROUTER_BUSY, DNET)
                .await;
            rig.reject(&ROUTER, RejectMessageReason::ROUTER_BUSY, DNET + 1)
                .await;
            rig.reject_as(Form::Global, RejectMessageReason::ROUTER_BUSY)
                .await;
        })
        .await;
    assert!(plain_timeout(&error), "{error:?}");
    rig.stop().await;
}

/// A busy reject tied to a request its caller then gave up on stays with
/// that request: the next request on the path, once the hold the first
/// left has passed, times out naming no router.
#[tokio::test(start_paused = true)]
async fn a_busy_reject_for_an_earlier_request_is_not_the_next_ones() {
    let mut rig = Rig::new().await;
    let task = rig.routed(1);
    rig.next().await;
    rig.reject(&ROUTER, RejectMessageReason::ROUTER_BUSY, DNET)
        .await;
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;
    let error = rig.unanswered(async |_: &Rig| {}).await;
    assert!(plain_timeout(&error), "{error:?}");
    rig.stop().await;
}
