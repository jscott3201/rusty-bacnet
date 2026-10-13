//! The checker's bookkeeping, driven directly.

use std::cell::RefCell;
use std::rc::Rc;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::MstpCoreConfig;

use super::Checker;
use crate::line::{Heard, Line};
use crate::model::{encode, Params, Seen};
use crate::queues::QueueLog;

fn poll_from(source: u8) -> Seen {
    Seen {
        frame_type: FrameType::POLL_FOR_MASTER,
        destination: 1,
        source,
        data: Vec::new(),
    }
}

/// A Reply To Poll For Master deferred at its start waits again, and the
/// core may drop it at its limit with nothing on the line. A deadline
/// serviced past that limit lets it go, so the station is free to answer
/// the next poll, which it then owes.
#[test]
fn a_deferred_answer_dropped_at_its_limit_frees_the_station() {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = 1;
    config.max_master = 15;
    let params = Params::new(&config);
    let mut storage = vec![0; 600];
    let line = Line::new(&mut storage, false, params.frame_abort);
    let mut checker = Checker::new(params, line, Rc::new(RefCell::new(QueueLog::default())));

    checker.heard(vec![Heard::Frame(poll_from(11))], 10_000);
    let answer = Seen {
        frame_type: FrameType::REPLY_TO_POLL_FOR_MASTER,
        destination: 11,
        source: 1,
        data: Vec::new(),
    };
    checker.handover(&encode(&answer), 15_000);
    assert!(checker.owed[0].handed);
    // A noise octet lands before the start, which the core refuses.
    checker.line.octet(0x00, 15_500, &mut Vec::new());
    checker.deferred(16_000);
    // Noise keeps the line busy past the answer's limit, about 18 ms after
    // the poll; then a deadline is serviced.
    checker.line.octet(0x00, 40_000, &mut Vec::new());
    checker.answers_due(50_000, None);
    assert!(checker.owed[0].released);

    checker.heard(vec![Heard::Frame(poll_from(12))], 60_000);
    let owed = checker.owed.last().expect("the second poll");
    assert_eq!(owed.from, 12);
    assert!(
        owed.required,
        "a free station owes the second poll an answer"
    );
}
