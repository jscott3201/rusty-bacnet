//! When the client answers a What-Is-Network-Number (#2597). It is not a
//! router, so Clause 6.4.19 lets it hold the answer to a broadcast
//! question for a random wait and skip it when another node answers first:
//! one question on a segment full of clients then draws one answer, not one
//! from each. A question sent to the client alone is answered at once.
use super::asking::{Site, open_site};
use super::*;
use bacnet_network::network_number::{ANSWER_DELAY_MAX, ANSWER_DELAY_MIN};
use bacnet_transport::test_transport::SentFrame;
use tokio::time::Duration;

/// A step on the paused clock that gives the client's tasks a turn.
const TURN: Duration = Duration::from_millis(1);

/// A client that has learned network 7 from an announcement with `flag`.
async fn taught(flag: u8) -> Site {
    let site = open_site(true).await;
    site.announce(7, flag).await;
    assert!(site.sent.is_empty(), "learning sends nothing");
    site
}

/// Ask the client, by broadcast or alone, and let it take the question in.
async fn ask(site: &Site, broadcast: bool) {
    inject(&site.inbound, QUERY, broadcast).await;
    tokio::time::sleep(TURN).await;
}

/// Have another node broadcast that this network is `network`, with `flag`.
async fn hear(site: &Site, network: u16, flag: u8) {
    inject(&site.inbound, &number(network, flag), true).await;
    tokio::time::sleep(TURN).await;
}

/// `frames` is one Network-Number-Is for network 7 as a learned number, a
/// local broadcast.
fn assert_one_answer(frames: &[SentFrame]) {
    assert_eq!(frames.len(), 1, "one answer: {frames:?}");
    assert!(frames[0].broadcast && frames[0].mac.is_empty());
    assert_eq!(frames[0].npdu.as_ref(), number(7, 0));
}

/// A broadcast question is answered inside the window the clause allows,
/// never at once, and once.
#[tokio::test(start_paused = true)]
async fn a_broadcast_question_is_answered_after_the_wait() {
    let mut site = taught(0).await;
    ask(&site, true).await;
    tokio::time::sleep(ANSWER_DELAY_MIN - TURN * 2).await;
    assert!(site.sent.is_empty(), "nothing before the shortest wait");
    tokio::time::sleep(ANSWER_DELAY_MAX - ANSWER_DELAY_MIN + TURN * 2).await;
    assert_one_answer(&site.sent.frames());
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(site.sent.len(), 1, "and no second one");
    site.client.stop().await.unwrap();
}

/// A question sent to the client alone is answered at once, with the same
/// broadcast, so it settles a broadcast question still waiting.
#[tokio::test(start_paused = true)]
async fn a_question_to_the_client_alone_is_answered_at_once() {
    let mut site = taught(0).await;
    ask(&site, false).await;
    assert_one_answer(&site.sent.take());
    ask(&site, true).await;
    ask(&site, false).await;
    assert_one_answer(&site.sent.take());
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert!(site.sent.is_empty(), "the waiting answer was settled");
    site.client.stop().await.unwrap();
}

/// Questions that come while an answer waits share it.
#[tokio::test(start_paused = true)]
async fn two_questions_in_the_wait_get_one_answer() {
    let mut site = taught(0).await;
    ask(&site, true).await;
    tokio::time::sleep(ANSWER_DELAY_MIN / 2).await;
    ask(&site, true).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 3).await;
    assert_one_answer(&site.sent.frames());
    // The wait over, a new question gets its own answer.
    ask(&site, true).await;
    tokio::time::sleep(ANSWER_DELAY_MAX + TURN).await;
    assert_eq!(site.sent.len(), 2);
    site.client.stop().await.unwrap();
}

/// Another node's answer heard in the wait stands for the client's own,
/// whether flagged configured or not: the client's would say no more.
#[tokio::test(start_paused = true)]
async fn an_answer_heard_in_the_wait_stands_for_the_clients() {
    for flag in [0, 1] {
        let mut site = taught(0).await;
        ask(&site, true).await;
        hear(&site, 7, flag).await;
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert!(site.sent.is_empty(), "heard flag {flag}: no answer");
        // Only that one answer was settled: the next question waits anew.
        ask(&site, true).await;
        tokio::time::sleep(ANSWER_DELAY_MAX + TURN).await;
        assert_eq!(site.sent.len(), 1, "heard flag {flag}");
        site.client.stop().await.unwrap();
    }
}

/// A heard answer is applied first, by Clause 6.4.20's precedence, and
/// compared after. One the client takes over (a configured number over
/// its learned one) puts it in agreement, and it stays quiet. One it
/// refuses (a learned number against one learned from a configured
/// announcement) leaves the two apart, and the client still answers with
/// its own, so the asker hears both.
#[tokio::test(start_paused = true)]
async fn a_differing_answer_counts_only_once_the_client_agrees() {
    let mut adopts = taught(0).await;
    ask(&adopts, true).await;
    hear(&adopts, 8, 1).await;
    assert_eq!(adopts.client.network_number(), Some(8));
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert!(adopts.sent.is_empty(), "it agrees now, and stays quiet");
    adopts.client.stop().await.unwrap();

    let mut keeps = taught(1).await;
    ask(&keeps, true).await;
    hear(&keeps, 8, 0).await;
    assert_eq!(keeps.client.network_number(), Some(7));
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_one_answer(&keeps.sent.frames());
    keeps.client.stop().await.unwrap();
}

/// A client that doesn't know the number has no answer, at once or later,
/// and learning the number in the meantime doesn't make one.
#[tokio::test(start_paused = true)]
async fn a_client_without_the_number_stays_quiet() {
    let mut site = open_site(true).await;
    ask(&site, true).await;
    ask(&site, false).await;
    site.announce(7, 1).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert!(site.sent.is_empty());
    site.client.stop().await.unwrap();
}

/// Stopping the client in the wait drops the answer with its owner task:
/// nothing goes out, then or after the wait would have ended.
#[tokio::test(start_paused = true)]
async fn stopping_in_the_wait_sends_nothing() {
    let mut site = taught(0).await;
    ask(&site, true).await;
    site.client.stop().await.unwrap();
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert!(site.sent.is_empty(), "stopped: {:?}", site.sent.frames());

    // Dropped without a stop, the same.
    let site = taught(0).await;
    ask(&site, true).await;
    let sent = site.sent.clone();
    drop(site);
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert!(sent.is_empty(), "dropped: {:?}", sent.frames());
}
