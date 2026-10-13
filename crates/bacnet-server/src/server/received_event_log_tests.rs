//! The per-source allowance on received notifications logged (#1346), on
//! hand-set instants: offsets from one reading of the server's timer clock
//! ([`runtime_clock::now`]), the one the receive path reads (#1550).
use super::*;

fn peer(n: u32) -> CanonicalPeer {
    CanonicalPeer::direct(&n.to_be_bytes())
}

/// How many of `count` records from `source` at `now` the allowances take.
fn taken(log: &ReceivedEventLog, source: &CanonicalPeer, now: Instant, count: u32) -> u32 {
    (0..count).filter(|_| log.admit(source, now)).count() as u32
}

#[test]
fn each_source_gets_its_rate_in_each_window() {
    let log = ReceivedEventLog::default();
    let start = runtime_clock::now();
    assert_eq!(taken(&log, &peer(1), start, 8), RECEIVED_EVENT_LOG_RATE);
    // Another source's allowance is its own.
    assert_eq!(taken(&log, &peer(2), start, 8), RECEIVED_EVENT_LOG_RATE);
    // Still the same window just before it runs out.
    let late = start + WINDOW - Duration::from_millis(1);
    assert_eq!(taken(&log, &peer(1), late, 1), 0);
    // The next window gives the rate again, and no more.
    assert_eq!(
        taken(&log, &peer(1), start + WINDOW, 8),
        RECEIVED_EVENT_LOG_RATE
    );
}

/// While the table is full of sources still inside their window, every
/// other source shares one more allowance: however many there are, the
/// logs take no more than (sources + 1) x rate records in a window.
#[test]
fn a_flood_of_sources_shares_one_allowance_past_the_table() {
    let log = ReceivedEventLog::default();
    let now = runtime_clock::now();
    let tracked = RECEIVED_EVENT_LOG_SOURCES as u32;
    let mut total = 0;
    for n in 0..tracked {
        total += taken(&log, &peer(n), now, 8);
    }
    assert_eq!(total, tracked * RECEIVED_EVENT_LOG_RATE);
    let mut untracked = 0;
    for n in tracked..tracked + 1000 {
        untracked += taken(&log, &peer(n), now, 8);
    }
    assert_eq!(untracked, RECEIVED_EVENT_LOG_RATE);
    assert_eq!(
        log.0.lock().unwrap().sources.len(),
        RECEIVED_EVENT_LOG_SOURCES
    );
    // A tracked source keeps its own window meanwhile.
    assert_eq!(taken(&log, &peer(0), now, 1), 0);
}

/// Once a tracked source has sent nothing for a whole window, a new source
/// takes its place, with a fresh allowance of its own; a source that sent
/// within the last window keeps its place.
#[test]
fn a_new_source_takes_the_place_of_one_silent_for_a_window() {
    let log = ReceivedEventLog::default();
    let start = runtime_clock::now();
    let tracked = RECEIVED_EVENT_LOG_SOURCES as u32;
    for n in 0..tracked {
        assert_eq!(taken(&log, &peer(n), start, 1), 1);
    }
    let later = start + WINDOW;
    // Peer 1 sends again in a new window; peer 0 stays silent.
    for n in 1..tracked {
        assert_eq!(taken(&log, &peer(n), later, 1), 1);
    }
    let newcomer = peer(tracked);
    assert_eq!(taken(&log, &newcomer, later, 8), RECEIVED_EVENT_LOG_RATE);
    let allowances = log.0.lock().unwrap();
    assert!(allowances.sources.iter().any(|(p, _)| *p == newcomer));
    assert!(allowances.sources.iter().all(|(p, _)| *p != peer(0)));
    assert_eq!(allowances.shared.taken, 0);
}

/// Over many windows of a flood from ever new sources, the logs never take
/// more than (sources + 1) x rate records in a window.
#[test]
fn the_ceiling_holds_window_after_window() {
    let log = ReceivedEventLog::default();
    let start = runtime_clock::now();
    let ceiling = (RECEIVED_EVENT_LOG_SOURCES as u32 + 1) * RECEIVED_EVENT_LOG_RATE;
    let mut next = 0;
    for window in 0..5 {
        let now = start + WINDOW * window;
        let mut total = 0;
        for _ in 0..500 {
            total += taken(&log, &peer(next), now, 3);
            next += 1;
        }
        assert!(total <= ceiling, "window {window}: {total}");
        assert!(total > 0);
    }
}

/// A source that sent late in its window keeps its place once that window
/// runs out: a new source has to wait a whole window after the last
/// notification of the source it replaces, and shares the spare allowance
/// until then.
#[test]
fn a_source_that_sent_late_in_its_window_keeps_its_place() {
    let log = ReceivedEventLog::default();
    let start = runtime_clock::now();
    let late = start + Duration::from_millis(900);
    let tracked = RECEIVED_EVENT_LOG_SOURCES as u32;
    for n in 0..tracked {
        assert_eq!(taken(&log, &peer(n), start, 1), 1);
        assert_eq!(taken(&log, &peer(n), late, 1), 1);
    }
    let newcomer = peer(tracked);
    assert_eq!(taken(&log, &newcomer, start + WINDOW, 1), 1);
    {
        let allowances = log.0.lock().unwrap();
        assert!(allowances.sources.iter().all(|(p, _)| *p != newcomer));
        assert!(allowances.sources.iter().any(|(p, _)| *p == peer(0)));
        assert_eq!(allowances.shared.taken, 1);
    }
    // A whole window after their last notification, the quiet ones give way.
    assert_eq!(taken(&log, &newcomer, late + WINDOW, 1), 1);
    let allowances = log.0.lock().unwrap();
    assert!(allowances.sources.iter().any(|(p, _)| *p == newcomer));
    assert_eq!(allowances.sources.len(), RECEIVED_EVENT_LOG_SOURCES);
}
