## Calls in per request, rp/mt: B - A (total over the 5 functions that moved by 0.5 or more: -10.0; the largest 5 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 15.28 | 13.26 | -2.01 | `clock_gettime@@GLIBC_2.17` |
| 11.15 | 9.14 | -2.01 | `<std::sys::pal::unix::time::Timespec>::now` |
| 11.17 | 9.16 | -2.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 2.00 | 0.00 | -2.00 | `<bacnet_time::host::HostClock>::now` |
| 2.00 | 0.00 | -2.00 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |

## Self instructions per request, rp/mt: B - A (total over the 15 functions that moved by 0.5 or more: -249.1; the largest 15 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 234.01 | 0.01 | -234.00 | `<bacnet_time::host::HostClock>::now` |
| 532.00 | 736.00 | +204.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 846.38 | 721.20 | -125.18 | `<std::sys::pal::unix::time::Timespec>::now` |
| 357.38 | 293.12 | -64.26 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 152.76 | 132.63 | -20.13 | `clock_gettime@@GLIBC_2.17` |
| 4.00 | 0.00 | -4.00 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 51.94 | 48.36 | -3.58 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 197.40 | 195.78 | -1.62 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 129.17 | 128.02 | -1.15 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 212.63 | 211.68 | -0.95 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 379.05 | 378.21 | -0.84 | `<tokio::runtime::time::Driver>::park_internal` |
| 101.08 | 101.85 | +0.77 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 49.15 | 49.90 | +0.75 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 82.77 | 83.36 | +0.59 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 378.32 | 378.84 | +0.52 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

