## Calls in per request, wp/mt: B - A (total over the 6 functions that moved by 0.5 or more: -8.0; the largest 6 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 11.13 | 9.13 | -2.00 | `<std::sys::pal::unix::time::Timespec>::now` |
| 15.25 | 13.25 | -2.00 | `clock_gettime@@GLIBC_2.17` |
| 10.15 | 9.15 | -1.00 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 2.00 | 1.00 | -1.00 | `<bacnet_server::cov::CovSubscriptionTable>::timed_capture` |
| 1.00 | 0.00 | -1.00 | `<bacnet_time::host::HostClock>::now` |
| 1.00 | 0.00 | -1.00 | `<std::time::Instant>::now` |

## Self instructions per request, wp/mt: B - A (total over the 13 functions that moved by 0.5 or more: -157.8; the largest 13 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 813.16 | 723.19 | -89.97 | `<std::sys::pal::unix::time::Timespec>::now` |
| 35.01 | 0.01 | -35.00 | `<bacnet_time::host::HostClock>::now` |
| 324.93 | 292.89 | -32.04 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 152.55 | 132.54 | -20.01 | `clock_gettime@@GLIBC_2.17` |
| 43.00 | 63.00 | +20.00 | `<bacnet_server::cov::CovSubscriptionTable>::timed_capture` |
| 50.68 | 56.99 | +6.31 | `_int_free_maybe_consolidate.part.0` |
| 470.49 | 465.47 | -5.02 | `_int_malloc` |
| 480.15 | 485.05 | +4.90 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 744.28 | 742.28 | -2.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 2.00 | 0.00 | -2.00 | `<std::time::Instant>::now` |
| 8.27 | 6.45 | -1.82 | `unlink_chunk.isra.0` |
| 21.65 | 19.91 | -1.73 | `_int_free_create_chunk` |
| 143.96 | 144.49 | +0.54 | `malloc_consolidate` |

