//! When the server answers a What-Is-Network-Number (#2597). It is not a
//! router, so Clause 6.4.19 lets it hold the answer to a broadcast question
//! for a random wait and skip it when another node answers first. A
//! question sent to the server alone is answered at once. A server
//! configured with its number (a registered Network Port) answers with the
//! configured flag, which a learned answer doesn't stand in for.
use super::*;
use bacnet_network::network_number::{ANSWER_DELAY_MAX, ANSWER_DELAY_MIN};

const QUERY: &[u8] = &[1, 0x80, 0x12];
/// No answer.
const NONE: [(u16, u8); 0] = [];
/// A step on the paused clock that gives the server's tasks a turn.
const TURN: Duration = Duration::from_millis(1);

fn number_is(number: u16, flag: u8) -> Vec<u8> {
    let [high, low] = number.to_be_bytes();
    vec![1, 0x80, 0x13, high, low, flag]
}

/// A server whose registered port is configured with this network's number.
async fn configured() -> Started {
    Started::new(database(Vec::new()), Some(THIS_NETWORK)).await
}

/// A server with no port, taught this network's number by an announcement
/// with `flag`.
async fn taught(flag: u8) -> Started {
    let started = Started::new(database(Vec::new()), None).await;
    assert_eq!(started.announce(THIS_NETWORK, flag).await, THIS_NETWORK);
    started
}

impl Started {
    /// Hand the server one control, by link broadcast when `group`, and let
    /// its worker take it in.
    async fn control(&self, npdu: &[u8], group: bool) {
        self.feed(npdu, group).await;
        tokio::time::sleep(TURN).await;
    }

    /// The Network-Number-Is broadcasts sent since the last look, as
    /// (number, flag).
    fn answers(&self) -> Vec<(u16, u8)> {
        self.sent
            .take()
            .iter()
            .map(|frame| {
                assert!(frame.broadcast, "a local broadcast: {frame:?}");
                assert_eq!(frame.npdu[..3], [1, 0x80, 0x13], "{frame:?}");
                assert_eq!(frame.npdu.len(), 6);
                (
                    u16::from_be_bytes([frame.npdu[3], frame.npdu[4]]),
                    frame.npdu[5],
                )
            })
            .collect()
    }
}

/// A broadcast question is answered inside the window the clause allows,
/// never at once, and once; questions that come in the wait share the
/// answer.
#[tokio::test(start_paused = true)]
async fn a_broadcast_question_is_answered_once_after_the_wait() {
    for (started, flag) in [(configured().await, 1), (taught(0).await, 0)] {
        started.control(QUERY, true).await;
        tokio::time::sleep(ANSWER_DELAY_MIN / 2).await;
        started.control(QUERY, true).await;
        tokio::time::sleep(ANSWER_DELAY_MIN / 2 - TURN * 3).await;
        assert_eq!(started.answers(), NONE, "nothing before the shortest wait");
        tokio::time::sleep(ANSWER_DELAY_MAX - ANSWER_DELAY_MIN + TURN * 3).await;
        assert_eq!(started.answers(), [(THIS_NETWORK, flag)]);
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert_eq!(started.answers(), NONE, "one answer for both questions");
        started.stop().await;
    }
}

/// A question sent to the server alone is answered at once, with the same
/// broadcast, so it settles a broadcast question still waiting.
#[tokio::test(start_paused = true)]
async fn a_question_to_the_server_alone_is_answered_at_once() {
    let started = configured().await;
    started.control(QUERY, true).await;
    started.control(QUERY, false).await;
    assert_eq!(started.answers(), [(THIS_NETWORK, 1)]);
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(started.answers(), NONE, "the waiting answer was settled");
    started.stop().await;
}

/// A learned number's answer says nothing a heard one doesn't, with or
/// without the configured flag: either stands for it.
#[tokio::test(start_paused = true)]
async fn any_agreeing_answer_stands_for_a_learned_one() {
    for (learned_from, heard) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let started = taught(learned_from).await;
        started.control(QUERY, true).await;
        started.control(&number_is(THIS_NETWORK, heard), true).await;
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert_eq!(
            started.answers(),
            NONE,
            "learned from flag {learned_from}, heard flag {heard}"
        );
        // Only that answer was settled: the next question waits anew.
        started.control(QUERY, true).await;
        tokio::time::sleep(ANSWER_DELAY_MAX + TURN).await;
        assert_eq!(started.answers(), [(THIS_NETWORK, 0)]);
        started.stop().await;
    }
}

/// A configured server's answer carries the configured flag, which lets
/// the asker hold the number as configured (Clause 6.4.20). Another
/// configured answer for the same number stands for it; a learned one, or
/// one for another number, doesn't, and the port keeps its configuration.
#[tokio::test(start_paused = true)]
async fn only_a_configured_agreeing_answer_stands_for_a_configured_one() {
    let started = configured().await;
    started.control(QUERY, true).await;
    started.control(&number_is(THIS_NETWORK, 1), true).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(started.answers(), NONE, "a configured answer was heard");

    for (number, flag) in [(THIS_NETWORK, 0), (8, 0), (8, 1)] {
        started.control(QUERY, true).await;
        started.control(&number_is(number, flag), true).await;
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert_eq!(
            started.answers(),
            [(THIS_NETWORK, 1)],
            "heard {number} flag {flag}"
        );
        assert_eq!(started.port_number().await, (THIS_NETWORK, 3));
    }
    started.stop().await;
}

/// A heard answer is applied first and compared after: a learned server
/// that takes a configured number over agrees with it, and stays quiet.
#[tokio::test(start_paused = true)]
async fn an_answer_the_server_takes_over_stands_for_its_own() {
    let started = taught(0).await;
    started.control(QUERY, true).await;
    started.control(&number_is(8, 1), true).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(started.answers(), NONE);
    started.control(QUERY, false).await;
    assert_eq!(started.answers(), [(8, 0)]);
    started.stop().await;
}

/// A server that doesn't know the number has no answer, at once or later.
#[tokio::test(start_paused = true)]
async fn a_server_without_the_number_stays_quiet() {
    for port in [None, Some(0)] {
        let started = Started::new(database(Vec::new()), port).await;
        started.control(QUERY, true).await;
        started.control(QUERY, false).await;
        started.control(&number_is(THIS_NETWORK, 1), true).await;
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert_eq!(started.answers(), NONE, "port {port:?}");
        started.stop().await;
    }
}

/// Stopping the server in the wait drops the answer with its Number task:
/// nothing goes out, then or after the wait would have ended.
#[tokio::test(start_paused = true)]
async fn stopping_in_the_wait_sends_nothing() {
    for started in [configured().await, taught(0).await] {
        started.control(QUERY, true).await;
        let sent = started.sent.clone();
        started.stop().await;
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert!(sent.is_empty(), "{:?}", sent.frames());
    }
}

/// An answer that came due while the database was busy is not lost when a
/// control arrives first: the wait for the database is cut off for the
/// control, and the answer, still pending, goes out after it.
#[tokio::test(start_paused = true)]
async fn an_answer_cut_off_at_the_database_still_goes_out() {
    let started = configured().await;
    started.control(QUERY, true).await;
    let held = started.server.database().clone().write_owned().await;
    // The answer comes due and its owner queues for the database.
    tokio::time::sleep(ANSWER_DELAY_MAX + TURN).await;
    assert_eq!(started.answers(), NONE);
    // A control that settles nothing: a learned number against the
    // configured one. Taking it drops the owner's wait for the database.
    started.control(&number_is(8, 0), true).await;
    drop(held);
    tokio::time::sleep(TURN * 3).await;
    assert_eq!(started.answers(), [(THIS_NETWORK, 1)]);
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(started.answers(), NONE, "once");
    started.stop().await;
}

/// One answer seen on a shared segment: which node sent it, what it
/// carried, and how long after the question.
#[derive(Debug, PartialEq, Eq)]
struct Heard {
    node: usize,
    number: u16,
    flag: u8,
    after: Duration,
}

/// Put `nodes` on one segment, ask them all by broadcast, and carry every
/// answer to the other nodes until all waits are over. The nodes get the
/// question in order, one turn apart, so `waits` go to them in that order
/// (`queue_answer_waits`); time moves a millisecond at a time, so an answer
/// reaches the others before any later wait ends.
async fn ask_the_segment(nodes: &[Started], waits: &[u64]) -> Vec<Heard> {
    bacnet_network::network_number::queue_answer_waits(
        waits.iter().map(|seconds| Duration::from_secs(*seconds)),
    );
    let asked = tokio::time::Instant::now();
    for node in nodes {
        node.control(QUERY, true).await;
    }
    let mut heard = Vec::new();
    while asked.elapsed() < ANSWER_DELAY_MAX * 2 {
        tokio::time::sleep(TURN).await;
        for (index, node) in nodes.iter().enumerate() {
            for (number, flag) in node.answers() {
                heard.push(Heard {
                    node: index,
                    number,
                    flag,
                    after: asked.elapsed(),
                });
                for (other, peer) in nodes.iter().enumerate() {
                    if other != index {
                        peer.feed(&number_is(number, flag), true).await;
                    }
                }
            }
        }
    }
    heard
}

/// `after` is `seconds` after the question, give or take the turns the
/// nodes took the question in.
fn about(after: Duration, seconds: u64) -> bool {
    let wait = Duration::from_secs(seconds);
    (wait..wait + TURN * 10).contains(&after)
}

/// Several servers configured with the same number and no router: the one
/// whose wait ends first answers, and the others, hearing a configured
/// answer for their number, stay quiet.
#[tokio::test(start_paused = true)]
async fn configured_servers_on_a_segment_give_one_answer() {
    let nodes = [configured().await, configured().await, configured().await];
    let heard = ask_the_segment(&nodes, &[4, 2, 6]).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!((heard[0].node, heard[0].number, heard[0].flag), (1, 7, 1));
    assert!(about(heard[0].after, 2), "{heard:?}");
    for node in nodes {
        node.stop().await;
    }
}

/// A learned node that answers first doesn't silence a configured one,
/// whose answer says more (two answers); a configured one that answers
/// first silences the learned node (one answer).
#[tokio::test(start_paused = true)]
async fn a_learned_answer_first_leaves_the_configured_one() {
    let nodes = [configured().await, taught(1).await];
    let heard = ask_the_segment(&nodes, &[4, 2]).await;
    assert_eq!(heard.len(), 2, "{heard:?}");
    assert_eq!((heard[0].node, heard[0].number, heard[0].flag), (1, 7, 0));
    assert_eq!((heard[1].node, heard[1].number, heard[1].flag), (0, 7, 1));
    assert!(about(heard[0].after, 2) && about(heard[1].after, 4));

    let heard = ask_the_segment(&nodes, &[2, 4]).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!((heard[0].node, heard[0].number, heard[0].flag), (0, 7, 1));
    for node in nodes {
        node.stop().await;
    }
}
