## Calls in per request, rp/ct: B - A (total over the 6 functions that moved by 0.5 or more: -13.0; the largest 6 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 4.00 | 0.00 | -4.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 17.01 | 15.01 | -2.00 | `clock_gettime@@GLIBC_2.17` |
| 2.00 | 0.00 | -2.00 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}>` |
| 2.00 | 0.00 | -2.00 | `core::ptr::drop_glue::<alloc::vec::into_iter::IntoIter<bacnet_server::server::segmented_receive::ExpiredRequest>>` |
| 16.63 | 14.66 | -1.97 | `<std::sys::pal::unix::time::Timespec>::now` |
| 9.20 | 8.21 | -1.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |

## Self instructions per request, rp/ct: B - A (total over the 9 functions that moved by 0.5 or more: -271.8; the largest 9 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 170.23 | 0.00 | -170.23 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 997.68 | 926.72 | -70.96 | `<std::sys::pal::unix::time::Timespec>::now` |
| 634.44 | 697.53 | +63.09 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 58.06 | 0.00 | -58.06 | `core::ptr::drop_glue::<alloc::vec::into_iter::IntoIter<bacnet_server::server::segmented_receive::ExpiredRequest>>` |
| 170.09 | 150.06 | -20.03 | `clock_gettime@@GLIBC_2.17` |
| 18.02 | 0.00 | -18.02 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}>` |
| 178.18 | 187.54 | +9.36 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 504.87 | 498.52 | -6.35 | `<tokio::runtime::time::Driver>::park_internal` |
| 110.55 | 109.89 | -0.66 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |

