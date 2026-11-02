//! The Trend Log Multiple row tables are `const` items, which the compiler
//! fills in at build time, so a coverage run sees `rows` as never called.
//! Calling it here at run time checks the tables match what it builds and
//! lets per-file coverage count it (#2183).
use super::*;
use core::hint::black_box;

#[test]
fn the_row_tables_are_what_rows_builds_at_run_time() {
    assert_eq!(rows(black_box(Always)), POLLED);
    assert_eq!(rows(black_box(ReadOnly)), TRIGGERED);
    // The two tables differ only in Log_Interval's write capability.
    for (polled, triggered) in POLLED.iter().zip(&TRIGGERED) {
        assert_eq!(polled.property_identifier, triggered.property_identifier);
        if polled.property_identifier == P::LOG_INTERVAL {
            assert_eq!(polled.write_capability, Always);
            assert_eq!(triggered.write_capability, ReadOnly);
        } else {
            assert_eq!(polled, triggered);
        }
    }
}
