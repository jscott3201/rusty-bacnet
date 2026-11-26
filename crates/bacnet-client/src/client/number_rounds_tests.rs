//! The learning rules on their own: which announcements are answers, what
//! a window's answers decide, and how often cues ask (#2826).
use super::*;

const WINDOW: Duration = Duration::from_secs(6);

fn at(start: Instant, seconds: u64) -> Instant {
    start + Duration::from_secs(seconds)
}

/// A question that went at `at`, with `losses` controls lost before it.
fn asked(at: Instant, losses: u64) -> Option<Question> {
    Some(Question { at, losses })
}

/// Hear each `(number, flag)` one second apart from `start`, all inside the
/// window of a question asked at `start`, and close the window.
fn window_of(answers: &[(u16, u8)]) -> Decision {
    let start = Instant::now();
    let mut learning = Learning::new(WINDOW);
    for (second, &(number, flag)) in (0..).zip(answers) {
        let heard = learning.heard((0, number, flag), asked(start, 0), at(start, second));
        assert_eq!(heard, Heard::Answer, "{answers:?}");
    }
    assert!(!learning.due(at(start, 5)));
    assert_eq!(learning.due(at(start, 6)), !answers.is_empty());
    learning.close(0)
}

/// The window is the APDU timeout or the longest a node that is not a
/// router may hold its answer, whichever is longer, and a margin.
#[test]
fn the_window_outlasts_a_held_answer() {
    let margin = ANSWER_MARGIN;
    assert_eq!(window_for(Duration::ZERO), ANSWER_DELAY_MAX + margin);
    assert_eq!(
        window_for(Duration::from_millis(500)),
        ANSWER_DELAY_MAX + margin
    );
    assert_eq!(
        window_for(Duration::from_secs(30)),
        Duration::from_secs(30) + margin
    );
}

/// Answers that agree decide; configured ones outrank learned ones, and a
/// split among the deciding answers decides nothing.
#[test]
fn a_window_takes_one_number_at_the_highest_flag_heard() {
    let agreed = |number, flag| Decision::Agreed { number, flag };
    assert_eq!(window_of(&[]), Decision::Unanswered);
    assert_eq!(window_of(&[(7, 1)]), agreed(7, 1));
    assert_eq!(window_of(&[(7, 0), (7, 0)]), agreed(7, 0));
    assert_eq!(window_of(&[(7, 0), (7, 1)]), agreed(7, 1));
    // A configured answer outranks learned ones that name another number.
    assert_eq!(window_of(&[(8, 0), (7, 1), (8, 0)]), agreed(7, 1));
    // Two numbers at the deciding flag decide nothing.
    assert_eq!(window_of(&[(7, 1), (5, 1)]), Decision::Split);
    assert_eq!(window_of(&[(7, 0), (5, 0)]), Decision::Split);
    assert_eq!(window_of(&[(7, 1), (5, 0), (7, 1)]), agreed(7, 1));
    assert_eq!(window_of(&[(7, 0), (5, 0), (7, 1)]), agreed(7, 1));
}

/// A control lost after the question went may have been an answer naming
/// another number: the window decides nothing. Losses before the earliest
/// question it holds answers to don't count; two numbers heard still split.
#[test]
fn a_window_with_a_loss_decides_nothing() {
    let start = Instant::now();
    let close_with = |answers: &[u16], losses: u64| {
        let mut learning = Learning::new(WINDOW);
        for (second, &number) in (0..).zip(answers) {
            learning.heard((0, number, 1), asked(start, 3), at(start, second));
        }
        learning.close(losses)
    };
    assert_eq!(close_with(&[7], 3), Decision::Agreed { number: 7, flag: 1 });
    assert_eq!(close_with(&[7], 4), Decision::Lost);
    assert_eq!(close_with(&[7, 5], 4), Decision::Split);

    // A later question noted more losses; the earlier baseline holds.
    let mut learning = Learning::new(WINDOW);
    learning.heard((0, 7, 1), asked(start, 3), start);
    learning.heard((0, 7, 1), asked(at(start, 2), 5), at(start, 3));
    assert_eq!(learning.close(5), Decision::Lost);
}

/// Outside a window, an announcement naming the number held is `Same`, and
/// one naming another is a cue; past the window, answers are cues again.
#[test]
fn only_announcements_inside_a_window_are_answers() {
    let start = Instant::now();
    let mut learning = Learning::new(WINDOW);
    assert_eq!(
        learning.heard((7, 7, 1), None, start),
        Heard::Same,
        "the number held"
    );
    assert_eq!(
        learning.heard((7, 5, 1), None, start),
        Heard::Cue { ask: true }
    );
    learning.cue_asked(start);
    // A question at 10 s: the window runs to 16 s.
    let question = asked(at(start, 10), 0);
    assert_eq!(
        learning.heard((7, 5, 1), question, at(start, 15)),
        Heard::Answer
    );
    assert!(learning.due(at(start, 16)));
    assert_eq!(learning.close(0), Decision::Agreed { number: 5, flag: 1 });
    assert_eq!(
        learning.heard((7, 5, 1), question, at(start, 16)),
        Heard::Cue { ask: false },
        "past the window, inside the hold"
    );
}

/// A question asked while a window is open stretches it, so its own
/// answers count in the same window.
#[test]
fn a_later_question_stretches_the_open_window() {
    let start = Instant::now();
    let mut learning = Learning::new(WINDOW);
    assert_eq!(
        learning.heard((0, 7, 1), asked(start, 0), start),
        Heard::Answer
    );
    let later = asked(at(start, 4), 0);
    assert_eq!(
        learning.heard((0, 5, 1), later, at(start, 5)),
        Heard::Answer
    );
    assert!(
        !learning.due(at(start, 6)),
        "stretched past the first window"
    );
    assert_eq!(
        learning.heard((0, 5, 1), later, at(start, 8)),
        Heard::Answer
    );
    assert!(!learning.due(at(start, 9)));
    assert!(learning.due(at(start, 10)));
    assert_eq!(learning.close(0), Decision::Split);
}

/// Cues ask at once the first time, then once per hold: a minute,
/// doubling to ten, and starting over after ten quiet minutes. Only a
/// question that went takes the hold.
#[test]
fn cues_ask_once_per_hold() {
    let start = Instant::now();
    let mut learning = Learning::new(WINDOW);
    let mut cue = |second| {
        let now = at(start, second);
        match learning.heard((7, 5, 1), None, now) {
            Heard::Cue { ask: true } => {
                learning.cue_asked(now);
                true
            }
            Heard::Cue { ask: false } => false,
            heard => panic!("{heard:?}"),
        }
    };
    assert!(cue(0));
    assert!(!cue(59));
    assert!(cue(60), "the one-minute hold is over");
    assert!(!cue(179));
    assert!(cue(180), "two minutes");
    assert!(cue(420), "four");
    assert!(cue(900), "eight");
    assert!(!cue(1499));
    assert!(cue(1500), "ten, the most");
    assert!(!cue(2099));
    assert!(cue(2100), "still ten");
    // Ten quiet minutes past the end of the hold: back to one minute.
    assert!(cue(2700 + 600));
    assert!(!cue(3300 + 59));
    assert!(cue(3300 + 60));

    // A cue whose question never went takes no hold.
    let mut learning = Learning::new(WINDOW);
    for second in [0, 1, 2] {
        assert_eq!(
            learning.heard((7, 5, 1), None, at(start, second)),
            Heard::Cue { ask: true },
            "{second}"
        );
    }
}

/// The wait for a window's end is pending while none is open, and ends
/// when the open one closes.
#[tokio::test(start_paused = true)]
async fn the_wait_ends_when_the_window_closes() {
    let mut learning = Learning::new(WINDOW);
    let idle = tokio::time::timeout(Duration::from_secs(3600), learning.closes());
    assert!(idle.await.is_err(), "no window, no end");
    let start = Instant::now();
    assert_eq!(
        learning.heard((0, 7, 1), asked(start, 0), start),
        Heard::Answer
    );
    learning.closes().await;
    assert_eq!(start.elapsed(), WINDOW);
    assert_eq!(learning.close(0), Decision::Agreed { number: 7, flag: 1 });
}
