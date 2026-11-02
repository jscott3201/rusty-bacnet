//! The Trend Log row tables are `const` items, which the compiler fills in
//! at build time, so a coverage run sees `rows` as never called. Calling it
//! here at run time checks the tables match what it builds and lets per-file
//! coverage count it (#2183).
use super::*;
use core::hint::black_box;

#[test]
fn the_row_tables_are_what_rows_builds_at_run_time() {
    assert_eq!(rows(black_box(Always)), WRITABLE_INTERVAL);
    assert_eq!(rows(black_box(ReadOnly)), READ_ONLY_INTERVAL);
    // The two tables differ only in Log_Interval's write capability.
    for (writable, read_only) in WRITABLE_INTERVAL.iter().zip(&READ_ONLY_INTERVAL) {
        assert_eq!(writable.property_identifier, read_only.property_identifier);
        if writable.property_identifier == P::LOG_INTERVAL {
            assert_eq!(writable.write_capability, Always);
            assert_eq!(read_only.write_capability, ReadOnly);
        } else {
            assert_eq!(writable, read_only);
        }
    }
}
