//! Inputs that once failed the target on a correct core, replayed by
//! `cargo test`, which `fuzz.sh check` runs: every file saved in
//! `regressions/mstp_core/`, as libFuzzer wrote it, and inputs written out
//! here as values.

use std::fs;
use std::path::Path;

use arbitrary::{Arbitrary, Unstructured};

use crate::input::{
    Address, Decision, FrameSpec, Input, Kind, Late, Path as Via, Policy, Setup, Step, Timing,
    TransmitEvent,
};
use crate::run;

/// Each saved input runs clean.
#[test]
fn saved_inputs_run_clean() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions/mstp_core");
    let mut replayed = 0;
    for entry in fs::read_dir(&directory).expect("the regressions directory") {
        let path = entry.expect("a directory entry").path();
        let bytes = fs::read(&path).expect("a saved input");
        let input = Input::arbitrary_take_rest(Unstructured::new(&bytes))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        run(&input);
        replayed += 1;
    }
    assert!(replayed > 0, "no saved inputs in {}", directory.display());
}

/// A data request for station 0 from 10, its data.
fn request() -> FrameSpec {
    FrameSpec {
        kind: Kind::DataExpectingReply,
        to: Address::This,
        from: Address::Other(10),
        length: 3,
        pattern: 1,
    }
}

/// The same data request arrives twice before the adapter polls: the core
/// takes the first and discards the repeat while it is busy with it. The
/// request id it hands up is the first's, and abandoning it leaves nothing
/// owed, so a deadline serviced on a quiet line inside the reply window
/// (T_reply_delay 150 ms here, a partial frame's T_frame_abort) finds no
/// answer missing.
#[test]
fn an_abandoned_request_asked_twice_owes_nothing() {
    let input = Input {
        setup: Setup {
            station: 0,
            max_master: 1,
            max_info_frames: 0,
            baud: 0,
            extended: false,
            layout: 0,
            unicast_slots: 2,
            broadcast_slots: 1,
            short_rx: None,
            lazy: true,
            timing: Timing {
                reply_timeout: 0,
                usage_timeout: 0,
                frame_abort: 0,
                turnaround: 0,
                usage_delay: 0,
                reply_delay: 100,
                margin: 0,
            },
            policy: Policy {
                multiple_frame_ratio: 128,
                broadcast_unicast_ratio: 0,
                starvation_limit: 0,
                queue_max_age: 1_000,
            },
        },
        steps: vec![
            Step::Frame {
                frame: request(),
                path: Via::Validated { with_octets: false },
            },
            Step::Frame {
                frame: request(),
                path: Via::Validated { with_octets: false },
            },
            Step::Decide {
                pick: 0,
                decision: Decision::Abandoned,
                after: 0,
            },
            Step::Wait(140_000),
            Step::Cut {
                frame: FrameSpec {
                    kind: Kind::Token,
                    to: Address::Other(20),
                    from: Address::Other(10),
                    length: 0,
                    pattern: 0,
                },
                at: 0,
            },
            Step::Deadline {
                late: Late::OnTime,
                start_before_poll: false,
            },
        ],
    };
    run(&input);
}

/// Station 0 of 0 and 1 at 9600 baud, with the fixed queues.
fn small_station(lazy: bool) -> Setup {
    Setup {
        station: 0,
        max_master: 1,
        max_info_frames: 0,
        baud: 0,
        extended: false,
        layout: 1,
        unicast_slots: 2,
        broadcast_slots: 1,
        short_rx: None,
        lazy,
        timing: Timing {
            reply_timeout: 0,
            usage_timeout: 0,
            frame_abort: 0,
            turnaround: 0,
            usage_delay: 0,
            reply_delay: 0,
            margin: 0,
        },
        policy: Policy {
            multiple_frame_ratio: 128,
            broadcast_unicast_ratio: 0,
            starvation_limit: 0,
            queue_max_age: 1_000,
        },
    }
}

/// Every kind of call the target makes, stamped before the core's last
/// one, at three points: with a request waiting for its answer, with the
/// answer handed over, and once it is on the wire. The core takes each as
/// if stamped at its last time (#1684), and the checker holds it to that.
#[test]
fn calls_stamped_in_the_past_are_taken_at_the_latest_time() {
    for event in 0..10 {
        for lazy in [false, true] {
            let regress = Step::Regress { back: 700, event };
            let input = Input {
                setup: small_station(lazy),
                steps: vec![
                    Step::Frame {
                        frame: request(),
                        path: Via::Validated { with_octets: false },
                    },
                    Step::Wait(2_000),
                    Step::Regress { back: 700, event },
                    Step::Decide {
                        pick: 0,
                        decision: Decision::Data {
                            length: 3,
                            pattern: 9,
                        },
                        after: 10,
                    },
                    Step::Drain,
                    Step::Wait(500),
                    Step::Regress { back: 300, event },
                    Step::Start { after: 0 },
                    Step::Wait(100),
                    regress,
                    Step::Run { rounds: 12 },
                ],
            };
            run(&input);
        }
    }
}

/// Transmit events for an id the adapter doesn't hold, with no frame of the
/// station's about, with its answer handed over, and with it on the wire:
/// each is refused as stale and changes nothing, its counters included.
#[test]
fn transmit_events_for_ids_nobody_holds_change_nothing() {
    let bogus = || {
        [
            TransmitEvent::Start,
            TransmitEvent::Complete,
            TransmitEvent::Abort,
        ]
        .map(|event| Step::Bogus { event, id: 5 })
    };
    let mut steps = Vec::new();
    steps.extend(bogus());
    steps.push(Step::Frame {
        frame: request(),
        path: Via::Validated { with_octets: false },
    });
    steps.push(Step::Decide {
        pick: 0,
        decision: Decision::Postponed,
        after: 10,
    });
    steps.push(Step::Drain);
    steps.extend(bogus());
    steps.push(Step::Start { after: 0 });
    steps.extend(bogus());
    steps.push(Step::Run { rounds: 4 });
    run(&Input {
        setup: small_station(false),
        steps,
    });
}
