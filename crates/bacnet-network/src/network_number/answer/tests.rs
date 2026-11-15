use super::*;

fn learned(number: u16, flag: u8) -> NetworkNumber {
    let mut state = NetworkNumber::default();
    state.observe(number, flag);
    state
}

fn configured(number: u16) -> NetworkNumber {
    NetworkNumber::configured(number).unwrap()
}

/// Whether `answer` comes due within `wait` of now, on the paused clock.
async fn due_within(answer: &DelayedAnswer, wait: Duration) -> bool {
    tokio::time::timeout(wait, answer.due()).await.is_ok()
}

#[test]
fn the_wait_covers_the_window_and_never_leaves_it() {
    let spread = (ANSWER_DELAY_MAX - ANSWER_DELAY_MIN).as_millis() as u32;
    assert_eq!(delay_from(0), ANSWER_DELAY_MIN);
    assert_eq!(delay_from(spread), ANSWER_DELAY_MAX);
    assert_eq!(
        delay_from(spread + 1),
        ANSWER_DELAY_MIN,
        "wraps, never past"
    );
    for random in [1, 4_999, spread - 1, u32::MAX / 3, u32::MAX - 1, u32::MAX] {
        let delay = delay_from(random);
        assert!((ANSWER_DELAY_MIN..=ANSWER_DELAY_MAX).contains(&delay));
    }
    // Clause 6.4.19's allowance, which the window must stay inside.
    assert_eq!(ANSWER_DELAY_MAX, Duration::from_secs(10));
    assert!(ANSWER_DELAY_MIN > Duration::ZERO);
    // The OS draws differ from node to node: sixteen alike would be a
    // constant, not a draw.
    let first = answer_delay();
    assert!((0..16).any(|_| answer_delay() != first));
}

#[tokio::test(start_paused = true)]
async fn a_broadcast_question_is_answered_inside_the_window() {
    let mut answer = DelayedAnswer::new();
    assert!(!answer.asked(learned(7, 0), true), "not at once");
    assert!(answer.due.is_some());
    let asked = Instant::now();
    assert!(
        !due_within(&answer, ANSWER_DELAY_MIN - Duration::from_millis(1)).await,
        "never before the shortest wait"
    );
    assert!(answer.due.is_some(), "a wait cut short keeps the answer");
    answer.due().await;
    assert!(asked.elapsed() <= ANSWER_DELAY_MAX);
    // Due stays due until the caller takes it.
    assert!(due_within(&answer, Duration::ZERO).await);
    answer.taken();
    assert!(answer.due.is_none());
    assert!(!due_within(&answer, ANSWER_DELAY_MAX * 2).await);
}

#[tokio::test(start_paused = true)]
async fn a_second_question_keeps_the_first_deadline() {
    let mut answer = DelayedAnswer::new();
    answer.asked(configured(7), true);
    let first = answer.due;
    tokio::time::advance(Duration::from_millis(400)).await;
    assert!(!answer.asked(configured(7), true));
    assert_eq!(answer.due, first, "one timer, not a later one");
}

#[tokio::test(start_paused = true)]
async fn a_question_to_the_node_alone_is_answered_now_and_settles_a_pending_one() {
    let mut answer = DelayedAnswer::new();
    assert!(answer.asked(learned(7, 0), false));
    assert!(answer.due.is_none());
    answer.asked(learned(7, 0), true);
    assert!(answer.asked(learned(7, 0), false));
    assert!(answer.due.is_none(), "the answer sent now is a broadcast");
}

#[tokio::test(start_paused = true)]
async fn a_node_without_a_number_has_no_answer() {
    let mut answer = DelayedAnswer::new();
    for broadcast in [false, true] {
        assert!(!answer.asked(NetworkNumber::default(), broadcast));
        assert!(answer.due.is_none());
    }
}

#[test]
fn a_heard_answer_counts_when_it_says_as_much_as_ours() {
    // (ours, heard number, heard flag, says as much)
    let cases = [
        (learned(7, 0), 7, 0, true),
        (learned(7, 0), 7, 1, true),
        (learned(7, 1), 7, 0, true),
        (learned(7, 1), 7, 1, true),
        (configured(7), 7, 1, true),
        // A configured node's answer carries the flag; a learned one's
        // doesn't make up for it.
        (configured(7), 7, 0, false),
        // Another number never stands in for ours.
        (learned(7, 0), 8, 0, false),
        (learned(7, 1), 8, 0, false),
        (configured(7), 8, 1, false),
        (NetworkNumber::default(), 7, 1, false),
    ];
    for (ours, number, flag, expected) in cases {
        assert_eq!(
            says_as_much(ours, number, flag),
            expected,
            "{ours:?} hears {number} flag {flag}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_heard_answer_drops_the_pending_one_only_when_it_says_as_much() {
    let mut answer = DelayedAnswer::new();
    answer.asked(configured(7), true);
    answer.heard(configured(7), 7, 0);
    answer.heard(configured(7), 8, 1);
    assert!(
        answer.due.is_some(),
        "a learned or differing answer leaves it"
    );
    answer.heard(configured(7), 7, 1);
    assert!(answer.due.is_none());
    assert!(!due_within(&answer, ANSWER_DELAY_MAX * 2).await);
}

#[tokio::test(start_paused = true)]
async fn queued_waits_are_used_in_order_before_random_ones() {
    queue_answer_waits([Duration::from_secs(3), Duration::from_secs(1)]);
    let asked = Instant::now();
    for wait in [3, 1] {
        let mut answer = DelayedAnswer::new();
        answer.asked(configured(7), true);
        assert_eq!(answer.due, Some(asked + Duration::from_secs(wait)));
    }
    let mut answer = DelayedAnswer::new();
    answer.asked(configured(7), true);
    let wait = answer.due.unwrap() - asked;
    assert!((ANSWER_DELAY_MIN..=ANSWER_DELAY_MAX).contains(&wait));
}
