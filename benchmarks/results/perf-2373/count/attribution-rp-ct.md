## Calls in per request, rp/ct: B - A (total over the 5 functions that moved by 0.5 or more: -10.0; the largest 5 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 2.00 | 0.00 | -2.00 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 2.00 | 0.00 | -2.00 | `<bacnet_time::host::HostClock>::now` |
| 12.01 | 10.01 | -2.00 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 15.01 | 13.01 | -2.00 | `clock_gettime@@GLIBC_2.17` |
| 14.65 | 12.66 | -1.99 | `<std::sys::pal::unix::time::Timespec>::now` |

## Self instructions per request, rp/ct: B - A (total over the 11 functions that moved by 0.5 or more: -241.0; the largest 11 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 234.00 | 0.01 | -234.00 | `<bacnet_time::host::HostClock>::now` |
| 532.00 | 736.00 | +204.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 927.10 | 804.90 | -122.19 | `<std::sys::pal::unix::time::Timespec>::now` |
| 384.19 | 320.20 | -63.99 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 150.06 | 130.06 | -20.00 | `clock_gettime@@GLIBC_2.17` |
| 4.00 | 0.00 | -4.00 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 183.19 | 185.81 | +2.62 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 501.40 | 499.73 | -1.68 | `<tokio::runtime::time::Driver>::park_internal` |
| 137.40 | 136.74 | -0.66 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 377.80 | 377.23 | -0.57 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 468.45 | 467.92 | -0.53 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |

