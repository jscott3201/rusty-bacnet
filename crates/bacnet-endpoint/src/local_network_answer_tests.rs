//! When an endpoint session answers a What-Is-Network-Number (#2597). It is
//! not a router, so Clause 6.4.19 lets it hold the answer to a broadcast
//! question for a random wait and skip it when another node answers first.
//! A question sent to the session alone is answered at once. The session
//! shares the server's Number owner, whose rules for a configured number
//! are tested there; these cover the session's own task and egress.
use super::*;
use bacnet_network::network_number::{ANSWER_DELAY_MAX, ANSWER_DELAY_MIN};

const QUERY: &[u8] = &[1, 0x80, 0x12];
/// No answer.
const NONE: [u16; 0] = [];
/// A step on the paused clock that gives the session's tasks a turn.
const TURN: Duration = Duration::from_millis(1);

impl Endpoint {
    /// Hand the session one control from the peer, by link broadcast when
    /// `group`, and let its Number task take it in.
    async fn control(&self, npdu: &[u8], group: bool) {
        self.deliver(mac(PEER), Bytes::copy_from_slice(npdu), group)
            .await;
        tokio::time::sleep(TURN).await;
    }

    /// The frames sent since the last look, each checked to be a
    /// Network-Number-Is broadcast, as the numbers they carry.
    fn answers(&mut self) -> Vec<u16> {
        let mut numbers = Vec::new();
        while let Ok(sent) = self.sent.try_recv() {
            assert!(sent.link.is_empty(), "a link broadcast");
            assert_eq!(sent.npdu[..3], [1, 0x80, 0x13]);
            assert_eq!((sent.npdu.len(), sent.npdu[5]), (6, 0), "learned");
            numbers.push(u16::from_be_bytes([sent.npdu[3], sent.npdu[4]]));
        }
        numbers
    }
}

/// A broadcast question is answered inside the window the clause allows,
/// never at once, and once; questions that come in the wait share the
/// answer.
#[tokio::test(start_paused = true)]
async fn a_broadcast_question_is_answered_once_after_the_wait() {
    for role in [
        SessionRole::ClientOnly,
        SessionRole::ServerOnly,
        SessionRole::Both,
    ] {
        let mut endpoint = Endpoint::started(role, true).await;
        endpoint.control(QUERY, true).await;
        tokio::time::sleep(ANSWER_DELAY_MIN / 2).await;
        endpoint.control(QUERY, true).await;
        tokio::time::sleep(ANSWER_DELAY_MIN / 2 - TURN * 3).await;
        assert_eq!(endpoint.answers(), NONE, "nothing before the shortest wait");
        tokio::time::sleep(ANSWER_DELAY_MAX - ANSWER_DELAY_MIN + TURN * 3).await;
        assert_eq!(endpoint.answers(), [THIS_NETWORK]);
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert_eq!(endpoint.answers(), NONE, "one answer for both questions");
        endpoint.stop().await;
    }
}

/// A question sent to the session alone is answered at once, with the same
/// broadcast, so it settles a broadcast question still waiting.
#[tokio::test(start_paused = true)]
async fn a_question_to_the_session_alone_is_answered_at_once() {
    let mut endpoint = Endpoint::started(SessionRole::Both, true).await;
    endpoint.control(QUERY, true).await;
    endpoint.control(QUERY, false).await;
    assert_eq!(endpoint.answers(), [THIS_NETWORK]);
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(endpoint.answers(), NONE, "the waiting answer was settled");
    endpoint.stop().await;
}

/// Another node's answer heard in the wait stands for the session's own
/// when it carries the session's number, at once or after the session took
/// a configured number over. A number the session refuses doesn't.
#[tokio::test(start_paused = true)]
async fn an_agreeing_answer_heard_in_the_wait_stands_for_the_sessions() {
    for (number, flag) in [(THIS_NETWORK, 0), (THIS_NETWORK, 1), (78, 1)] {
        let mut endpoint = Endpoint::started(SessionRole::Both, true).await;
        endpoint.control(QUERY, true).await;
        let [high, low] = number.to_be_bytes();
        endpoint
            .control(&[1, 0x80, 0x13, high, low, flag], true)
            .await;
        tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
        assert_eq!(endpoint.answers(), NONE, "heard {number} flag {flag}");
        // Only that answer was settled: the next question waits anew.
        endpoint.control(QUERY, true).await;
        tokio::time::sleep(ANSWER_DELAY_MAX + TURN).await;
        assert_eq!(endpoint.answers(), [number]);
        endpoint.stop().await;
    }

    // Learned from a configured announcement, the number stays when a
    // learned one differs (Clause 6.4.20), and the session answers with it.
    let mut endpoint = Endpoint::started(SessionRole::Both, true).await;
    endpoint.control(&[1, 0x80, 0x13, 0, 78, 1], true).await;
    endpoint.control(QUERY, true).await;
    endpoint.control(&[1, 0x80, 0x13, 0, 79, 0], true).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(endpoint.answers(), [78]);
    endpoint.stop().await;
}

/// Stopping the session in the wait drops the answer with its Number task:
/// nothing goes out, then or after the wait would have ended.
#[tokio::test(start_paused = true)]
async fn stopping_in_the_wait_sends_nothing() {
    let mut endpoint = Endpoint::started(SessionRole::Both, true).await;
    endpoint.control(QUERY, true).await;
    bounded(endpoint.session.stop()).await.unwrap();
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(endpoint.answers(), NONE);
}
