## Calls in per request, wp/mt: B - A (total over the 8 functions that moved by 0.5 or more: -3.4; the largest 8 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 4.00 | 0.00 | -4.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 49.00 | 52.00 | +3.00 | `__GI_memcpy` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 1.00 | 0.00 | -1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 0.00 | 1.00 | +1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 0.00 | 1.00 | +1.00 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |
| 0.96 | 1.53 | +0.57 | `_int_free_maybe_consolidate.part.0` |

## Self instructions per request, wp/mt: B - A (total over the 30 functions that moved by 0.5 or more: -471.5; the largest 25 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 521.00 | 0.00 | -521.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 240.00 | 0.00 | -240.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 0.00 | 175.00 | +175.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 58.00 | 190.00 | +132.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 108.00 | 0.00 | -108.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 1,603.00 | 1,651.00 | +48.00 | `__GI_memcpy` |
| 0.00 | 37.00 | +37.00 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |
| 26.00 | 0.00 | -26.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 461.07 | 478.34 | +17.27 | `_int_malloc` |
| 28.87 | 46.02 | +17.15 | `_int_free_maybe_consolidate.part.0` |
| 137.92 | 145.75 | +7.82 | `malloc_consolidate` |
| 27.84 | 22.99 | -4.86 | `_int_free_create_chunk` |
| 6.90 | 10.38 | +3.48 | `unlink_chunk.isra.0` |
| 3.83 | 6.14 | +2.31 | `alloc_perturb` |
| 127.02 | 124.82 | -2.20 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 945.44 | 943.62 | -1.82 | `<std::sys::pal::unix::time::Timespec>::now` |
| 212.72 | 211.02 | -1.70 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 1,272.71 | 1,271.28 | -1.43 | `free` |
| 425.70 | 427.03 | +1.33 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{clo` |
| 48.90 | 49.94 | +1.04 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 22.00 | 21.00 | -1.00 | `<bacnet_server::life_safety_cov::LifeSafetyCovSnapshots>::capture_write_property` |
| 357.80 | 357.02 | -0.78 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 1.58 | 0.84 | -0.75 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 1.29 | 0.61 | -0.69 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 374.46 | 373.78 | -0.68 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

