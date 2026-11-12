## Calls in per request, wp/ct: B - A (total over the 6 functions that moved by 0.5 or more: -8.0; the largest 6 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 15.01 | 13.01 | -2.00 | `clock_gettime@@GLIBC_2.17` |
| 14.66 | 12.66 | -2.00 | `<std::sys::pal::unix::time::Timespec>::now` |
| 2.00 | 1.00 | -1.00 | `<bacnet_server::cov::CovSubscriptionTable>::timed_capture` |
| 11.01 | 10.01 | -1.00 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 1.00 | 0.00 | -1.00 | `<bacnet_time::host::HostClock>::now` |
| 1.00 | 0.00 | -1.00 | `<std::time::Instant>::now` |

## Self instructions per request, wp/ct: B - A (total over the 8 functions that moved by 0.5 or more: -156.0; the largest 8 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 894.79 | 804.76 | -90.03 | `<std::sys::pal::unix::time::Timespec>::now` |
| 35.01 | 0.01 | -35.00 | `<bacnet_time::host::HostClock>::now` |
| 352.22 | 320.22 | -32.00 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 43.00 | 63.00 | +20.00 | `<bacnet_server::cov::CovSubscriptionTable>::timed_capture` |
| 150.07 | 130.07 | -20.00 | `clock_gettime@@GLIBC_2.17` |
| 480.00 | 485.00 | +5.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 744.24 | 742.24 | -2.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 2.00 | 0.00 | -2.00 | `<std::time::Instant>::now` |

