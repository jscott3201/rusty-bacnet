//! The Device row tables are `const` items, which the compiler fills in at
//! build time, so a coverage run sees `effective` as never called. Calling
//! it here at run time checks the tables match what it builds (#2183).
use super::*;
use core::hint::black_box;

#[test]
fn the_row_tables_are_what_effective_builds_at_run_time() {
    assert_eq!(
        &effective::<26>(black_box(false), black_box(false))[..],
        CLOCKLESS
    );
    assert_eq!(
        &effective::<28>(black_box(false), black_box(true))[..],
        CLOCKLESS_SEGMENTED
    );
    assert_eq!(
        &effective::<30>(black_box(true), black_box(false))[..],
        CLOCKED
    );
}
