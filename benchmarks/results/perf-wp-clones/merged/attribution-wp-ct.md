## Calls in per request, wp/ct: B - A (total over the 7 functions that moved by 0.5 or more: -4.0; the largest 7 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 4.00 | 0.00 | -4.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 49.00 | 52.00 | +3.00 | `__GI_memcpy` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 2.00 | 0.00 | -2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 1.00 | 0.00 | -1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 0.00 | 1.00 | +1.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 0.00 | 1.00 | +1.00 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |

## Self instructions per request, wp/ct: B - A (total over the 10 functions that moved by 0.5 or more: -504.8; the largest 10 listed)

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
| 22.00 | 21.00 | -1.00 | `<bacnet_server::life_safety_cov::LifeSafetyCovSnapshots>::capture_write_property` |
| 189.71 | 188.90 | -0.81 | `<tokio::runtime::time::handle::Handle>::process_at_time` |

