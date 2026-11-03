## Calls in per request, wp/ct: B - A (total over the 7 functions that moved by 0.5 or more: -4.0; the largest 7 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 12.00 | 8.00 | -4.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 42.00 | 45.00 | +3.00 | `__GI_memcpy` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 1.00 | 0.00 | -1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 0.00 | 1.00 | +1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 0.00 | 1.00 | +1.00 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |

## Self instructions per request, wp/ct: B - A (total over the 13 functions that moved by 0.5 or more: -503.2; the largest 13 listed)

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
| 180.58 | 188.75 | +8.17 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 503.74 | 497.89 | -5.85 | `<tokio::runtime::time::Driver>::park_internal` |
| 22.00 | 21.00 | -1.00 | `<bacnet_server::life_safety_cov::LifeSafetyCovSnapshots>::capture_write_property` |
| 1,025.78 | 1,024.83 | -0.95 | `<std::sys::pal::unix::time::Timespec>::now` |
| 110.41 | 109.82 | -0.59 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |

