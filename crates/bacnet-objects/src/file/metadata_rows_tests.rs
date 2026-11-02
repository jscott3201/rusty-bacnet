//! The File row tables are `const` items, which the compiler fills in at
//! build time, so a coverage run sees `with_resize` as never called. Calling
//! it here at run time checks the tables match what it builds (#2183).
use super::*;
use core::hint::black_box;

#[test]
fn the_row_tables_are_what_with_resize_builds_at_run_time() {
    assert_eq!(with_resize(black_box(FILE_SIZE_ROW)), STREAM_WRITABLE);
    assert_eq!(with_resize(black_box(RECORD_COUNT_ROW)), RECORD_WRITABLE);
    for (index, row) in BASE.iter().enumerate() {
        assert_eq!(STREAM_WRITABLE[index] == *row, index != FILE_SIZE_ROW);
        assert_eq!(RECORD_WRITABLE[index] == *row, index != RECORD_COUNT_ROW);
    }
}
