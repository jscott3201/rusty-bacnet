//! The window a question opens (#2826): how long it stays open, what makes
//! it decide nothing, and what neither a stop, a failed question nor one
//! cut off mid-send leaves behind.
use super::asking::{Gated, gated, open_site, open_site_with};
use super::local_routes::{REMOTE_NETWORK, ROUTER, THIS_NETWORK};
use super::*;
use crate::client::ClientConfig;
use bacnet_network::network_number::ANSWER_DELAY_MAX;
use bacnet_npdu::NetworkMessageType;
use bacnet_transport::test_transport::SendLog;
use tokio::time::Duration;

/// Past any window the clients here open: the longest a node that is not
/// a router may hold its answer, and room to spare.
const PAST_EVERY_WINDOW: Duration = Duration::from_secs(20);

/// A node that is not a router, which may hold its answer (#2597).
const NODE: [u8; 1] = [5];

/// The What-Is-Network-Number questions in `sent`.
fn questions(sent: &SendLog) -> usize {
    sent.frames()
        .iter()
        .filter(|frame| {
            frame.decode_npdu().message_type == Some(NetworkMessageType::WHAT_IS_NETWORK_NUMBER)
        })
        .count()
}

/// A node that is not a router may hold its answer for up to ten seconds
/// (Clause 6.4.19), so the window stays open at least that long, and a
/// little more, whatever the client's APDU timeout: a short timeout does
/// not cut it off, and the node's late answer counts.
#[tokio::test(start_paused = true)]
async fn a_late_answer_from_a_node_that_is_not_a_router_counts() {
    for apdu_timeout_ms in [500, 6_000] {
        let config = ClientConfig {
            apdu_timeout_ms,
            ..ClientConfig::default()
        };
        let mut site = open_site_with(true, config).await;
        site.client.what_is_network_number(None).await.unwrap();
        tokio::time::sleep(ANSWER_DELAY_MAX - Duration::from_millis(100)).await;
        broadcast_from(&site.inbound, &number(THIS_NETWORK, 0), &NODE).await;
        tokio::time::sleep(PAST_EVERY_WINDOW).await;
        assert_eq!(
            site.client.network_number(),
            Some(THIS_NETWORK),
            "APDU timeout {apdu_timeout_ms} ms"
        );
        assert_eq!(questions(&site.sent), 1, "the late answer was no cue");
        site.client.stop().await.unwrap();
    }
}

/// Hold the client's next question in its send: an unasked announcement
/// from `ROUTER` naming `REMOTE_NETWORK` is a cue, and the Number owner
/// waits in the send of the question it calls for.
async fn owner_held(site: &Gated) {
    broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &ROUTER).await;
    bounded(site.entered.acquire()).await.unwrap().forget();
}

/// While the window is open, more answers come than the Number owner's
/// queue holds, so some are turned away: what was turned away may have
/// named another number, so the window decides nothing, though every
/// answer the owner saw agreed. The next window, with nothing lost,
/// decides.
#[tokio::test(start_paused = true)]
async fn a_window_that_may_have_lost_an_answer_decides_nothing() {
    let mut site = gated().await;
    owner_held(&site).await;
    for _ in 0..300 {
        broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    }
    site.gate.add_permits(100);
    tokio::time::sleep(PAST_EVERY_WINDOW).await;
    assert_eq!(site.client.network_number(), None, "answers were lost");
    assert_eq!(site.client.network_number_disagreements(), 1, "counted");

    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(PAST_EVERY_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
    site.client.stop().await.unwrap();
}

/// A client stopped while its window is open takes nothing from it and
/// sends nothing more; the stop doesn't wait for the window.
#[tokio::test(start_paused = true)]
async fn a_stop_mid_window_takes_nothing() {
    let mut site = open_site(true).await;
    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let stopped = tokio::time::Instant::now();
    site.client.stop().await.unwrap();
    assert!(stopped.elapsed() < Duration::from_secs(1), "no wait for it");
    let sent = site.sent.len();
    tokio::time::sleep(PAST_EVERY_WINDOW).await;
    assert_eq!(site.client.network_number(), None);
    assert_eq!(site.sent.len(), sent, "nothing after the stop");
}

/// A cue whose question fails to send spends no hold: the next cue asks
/// at once.
#[tokio::test(start_paused = true)]
async fn a_cue_whose_question_fails_keeps_the_hold() {
    let mut site = open_site(true).await;
    site.handle.fail_next_send();
    broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    // The log keeps a send that failed.
    assert_eq!(questions(&site.sent), 1, "the question, which failed");
    broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(questions(&site.sent), 2, "asked again at once");
    site.client.stop().await.unwrap();
}

/// A question cut off in its send, its caller gone, opens no window: an
/// announcement after it is a cue, which the owner asks about, not an
/// answer.
#[tokio::test(start_paused = true)]
async fn a_question_cut_off_mid_send_opens_no_window() {
    let mut site = gated().await;
    let given_up = tokio::time::timeout(
        Duration::from_millis(100),
        site.client.what_is_network_number(None),
    );
    assert!(given_up.await.is_err(), "held in the send, then dropped");
    bounded(site.entered.acquire()).await.unwrap().forget();
    site.gate.add_permits(100);
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(site.asks(), 2, "the announcement was a cue");
    tokio::time::sleep(PAST_EVERY_WINDOW).await;
    assert_eq!(site.client.network_number(), None, "no answer was heard");
    site.client.stop().await.unwrap();
}

/// An answer heard while a caller's question is still in its send counts
/// toward that question's window; when the send is then cut off, the note
/// goes back, and the window it opened decides nothing (#2826): the
/// question may never have gone, and the "answer" may have been an
/// announcement nobody asked for.
#[tokio::test(start_paused = true)]
async fn a_window_whose_question_was_cut_off_decides_nothing() {
    let mut site = gated().await;
    {
        let asking = site.client.what_is_network_number(None);
        let held = tokio::time::timeout(Duration::from_millis(100), asking);
        let answer = async {
            bounded(site.entered.acquire()).await.unwrap().forget();
            broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
        };
        let (held, ()) = tokio::join!(held, answer);
        assert!(held.is_err(), "held in the send, then cut off");
    }
    site.gate.add_permits(100);
    tokio::time::sleep(PAST_EVERY_WINDOW).await;
    assert_eq!(site.client.network_number(), None, "no question went");
    site.client.stop().await.unwrap();
}

/// An answer that came inside the window counts even when the Number
/// owner only takes it from its queue after the window closed (#2826):
/// it is judged by when it came, not when it was handled. Here the owner
/// waits in the send of the question it asks about an announcement while
/// the router's answer waits in its queue past the close.
#[tokio::test(start_paused = true)]
async fn an_answer_queued_before_the_close_counts() {
    let mut site = gated().await;
    owner_held(&site).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(PAST_EVERY_WINDOW).await;
    assert_eq!(site.client.network_number(), None, "still held");
    site.gate.add_permits(100);
    // The router check before the number is taken: the APDU timeout.
    tokio::time::sleep(Duration::from_millis(6_100)).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
    site.client.stop().await.unwrap();
}

/// The longest APDU timeout a configuration can hold opens a window that
/// does not overflow the clock: the Number owner keeps running (#2826).
#[tokio::test(start_paused = true)]
async fn the_longest_apdu_timeout_does_not_stop_the_owner() {
    let config = ClientConfig {
        apdu_timeout_ms: u64::MAX,
        ..ClientConfig::default()
    };
    let mut site = open_site_with(true, config).await;
    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let owner = site.client.network_number_task.as_ref().expect("an owner");
    assert!(!owner.is_finished(), "the owner is still running");
    site.client.stop().await.unwrap();
}

/// A window doesn't close while answers that came before its close still
/// wait in the Number owner's queue (#2826): they count in it. Here the
/// router's answer is in, the owner then waits in the send of the question
/// a link return calls for, and a stranger's answer naming another number
/// comes and waits in the queue past the close. Both count in one window,
/// which decides nothing. Were the window closed first, the router's number
/// would be taken, then the stranger's, from a window of its own. Run a few
/// times over, since which of two ready turns the owner takes first is the
/// runtime's choice.
#[tokio::test(start_paused = true)]
async fn an_answer_waiting_in_the_queue_keeps_its_window_open() {
    const STRANGER: [u8; 1] = [66];
    for run in 0..8 {
        let mut site = gated().await;
        site.gate.add_permits(1);
        site.client.what_is_network_number(None).await.unwrap();
        bounded(site.entered.acquire()).await.unwrap().forget();
        broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        site.back.send_modify(|count| *count += 1);
        bounded(site.entered.acquire()).await.unwrap().forget();
        broadcast_from(&site.inbound, &number(THIS_NETWORK + 1, 1), &STRANGER).await;
        tokio::time::sleep(PAST_EVERY_WINDOW).await;
        site.gate.add_permits(100);
        tokio::time::sleep(PAST_EVERY_WINDOW).await;
        assert_eq!(site.client.network_number(), None, "run {run}");
        assert_eq!(site.client.network_number_disagreements(), 1, "run {run}");
        site.client.stop().await.unwrap();
    }
}
