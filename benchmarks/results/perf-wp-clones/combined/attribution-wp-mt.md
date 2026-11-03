## Calls in per request, wp/mt: B - A (total over the 7 functions that moved by 0.5 or more: -4.0; the largest 7 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 12.00 | 8.00 | -4.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 42.00 | 45.00 | +3.00 | `__GI_memcpy` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 1.00 | 0.00 | -1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 0.00 | 1.00 | +1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 0.00 | 1.00 | +1.00 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |

## Self instructions per request, wp/mt: B - A (total over the 16 functions that moved by 0.5 or more: -495.0; the largest 16 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 1,496.00 | 975.00 | -521.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 240.00 | 0.00 | -240.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 0.00 | 175.00 | +175.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 58.00 | 190.00 | +132.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 108.00 | 0.00 | -108.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 1,491.00 | 1,539.00 | +48.00 | `__GI_memcpy` |
| 0.00 | 37.00 | +37.00 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |
| 26.00 | 0.00 | -26.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 466.23 | 478.34 | +12.11 | `_int_malloc` |
| 46.97 | 41.24 | -5.74 | `_int_free_maybe_consolidate.part.0` |
| 7.28 | 10.65 | +3.36 | `unlink_chunk.isra.0` |
| 22.72 | 24.32 | +1.60 | `_int_free_create_chunk` |
| 126.61 | 125.37 | -1.24 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 22.00 | 21.00 | -1.00 | `<bacnet_server::life_safety_cov::LifeSafetyCovSnapshots>::capture_write_property` |
| 468.25 | 467.64 | -0.61 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 373.98 | 373.47 | -0.51 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

