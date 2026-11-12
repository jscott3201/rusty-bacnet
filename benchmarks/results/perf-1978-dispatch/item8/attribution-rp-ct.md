## Calls in per request, rp/ct: B - A (total over the 15 functions that moved by 0.5 or more: -35.4; the largest 15 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 7.72 | 4.20 | -3.52 | `tokio::runtime::task::waker::drop_waker` |
| 16.72 | 13.20 | -3.52 | `__aarch64_ldadd8_acq_rel` |
| 11.72 | 8.20 | -3.52 | `tokio::runtime::task::waker::clone_waker` |
| 83.39 | 79.88 | -3.52 | `__aarch64_ldadd8_relax` |
| 9.98 | 6.47 | -3.51 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 11.98 | 8.47 | -3.51 | `__aarch64_cas8_acq` |
| 39.06 | 35.55 | -3.51 | `__aarch64_cas8_acq_rel` |
| 3.51 | 0.00 | -3.51 | `<tokio::time::interval::Interval>::poll_tick` |
| 9.44 | 5.93 | -3.50 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 0.00 | 3.00 | +3.00 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 7.75 | 5.00 | -2.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 2.50 | 0.00 | -2.50 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 2.50 | 0.00 | -2.50 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 0.00 | 2.00 | +2.00 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closu` |
| 3.01 | 2.00 | -1.01 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |

## Self instructions per request, rp/ct: B - A (total over the 21 functions that moved by 0.5 or more: -770.5; the largest 21 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 398.69 | 224.00 | -174.69 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.50 | 296.39 | -172.11 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.67 | 0.00 | -117.67 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.94 | 0.00 | -87.94 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 377.25 | 289.81 | -87.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 76.70 | 0.02 | -76.68 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.00 | 72.00 | +72.00 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.00 | 63.00 | +63.00 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closu` |
| 108.05 | 58.77 | -49.28 | `tokio::runtime::task::waker::drop_waker` |
| 164.10 | 114.82 | -49.28 | `tokio::runtime::task::waker::clone_waker` |
| 93.28 | 62.00 | -31.28 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 83.60 | 66.01 | -17.60 | `__aarch64_ldadd8_acq_rel` |
| 416.96 | 399.38 | -17.58 | `__aarch64_ldadd8_relax` |
| 59.91 | 42.35 | -17.56 | `__aarch64_cas8_acq` |
| 195.30 | 177.74 | -17.56 | `__aarch64_cas8_acq_rel` |
| 865.32 | 878.81 | +13.48 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 189.63 | 181.78 | -7.85 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 497.05 | 502.31 | +5.26 | `<tokio::runtime::time::Driver>::park_internal` |
| 92.87 | 91.87 | -1.00 | `counts::main::{closure#0}` |
| 804.47 | 805.20 | +0.74 | `<std::sys::pal::unix::time::Timespec>::now` |
| 109.74 | 110.29 | +0.55 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |

