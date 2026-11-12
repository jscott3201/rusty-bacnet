## Calls in per request, rp/mt: B - A (total over the 15 functions that moved by 0.5 or more: -35.3; the largest 15 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 83.39 | 79.88 | -3.51 | `__aarch64_ldadd8_relax` |
| 46.03 | 42.53 | -3.51 | `__aarch64_cas8_acq_rel` |
| 9.97 | 6.47 | -3.51 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 14.97 | 11.47 | -3.51 | `__aarch64_cas8_acq` |
| 9.43 | 5.93 | -3.50 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 11.71 | 8.21 | -3.50 | `tokio::runtime::task::waker::clone_waker` |
| 7.71 | 4.21 | -3.50 | `tokio::runtime::task::waker::drop_waker` |
| 22.66 | 19.16 | -3.50 | `__aarch64_ldadd8_acq_rel` |
| 3.50 | 0.00 | -3.50 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.00 | 3.00 | +3.00 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 7.75 | 5.00 | -2.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 2.50 | 0.00 | -2.50 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 2.50 | 0.00 | -2.50 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 0.00 | 2.00 | +2.00 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closu` |
| 3.00 | 2.00 | -1.00 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |

## Self instructions per request, rp/mt: B - A (total over the 23 functions that moved by 0.5 or more: -767.2; the largest 23 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 398.54 | 224.00 | -174.54 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.05 | 296.29 | -171.76 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.54 | 0.00 | -117.54 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 378.43 | 290.62 | -87.81 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 87.74 | 0.00 | -87.74 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 76.43 | 0.03 | -76.40 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.00 | 72.00 | +72.00 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.00 | 63.00 | +63.00 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closu` |
| 164.00 | 114.96 | -49.04 | `tokio::runtime::task::waker::clone_waker` |
| 107.93 | 58.90 | -49.03 | `tokio::runtime::task::waker::drop_waker` |
| 93.15 | 62.00 | -31.15 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 416.93 | 399.40 | -17.53 | `__aarch64_ldadd8_relax` |
| 230.17 | 212.65 | -17.53 | `__aarch64_cas8_acq_rel` |
| 77.87 | 60.34 | -17.53 | `__aarch64_cas8_acq` |
| 113.31 | 95.80 | -17.51 | `__aarch64_ldadd8_acq_rel` |
| 865.33 | 878.76 | +13.43 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 130.31 | 128.06 | -2.25 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 422.27 | 423.57 | +1.30 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{clo` |
| 67.93 | 66.96 | -0.97 | `counts::main::{closure#0}` |
| 376.93 | 377.61 | +0.68 | `<tokio::runtime::time::Driver>::park_internal` |
| 193.74 | 194.38 | +0.64 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 46.33 | 46.94 | +0.61 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 211.82 | 211.27 | -0.55 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |

