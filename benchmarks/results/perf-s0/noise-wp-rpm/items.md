# #1978 items by self instructions per request (variant A)

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,629.0 | 1,052.7 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,577.7 | 892.0 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 830.9 | 300.8 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 804.9: `<std::sys::pal::unix::time::Timespec>::now`
- 384.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 171.3: `<std::time::Instant>::duration_since`
- 170.2: `clock_gettime@@GLIBC_2.17`
- 66.0: `<std::time::Instant>::elapsed`
- 28.0: `<std::time::Instant>::now`
- 4.0: `<tokio::time::instant::Instant>::now`
- 0.0: `<std::time::Instant>::checked_add`

Item 2a, largest functions:

- 636.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 452.6: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 186.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 125.2: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 52.0: `<tokio::sync::batch_semaphore::Semaphore>::try_acquire`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 40.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 35.0: `<tokio::sync::batch_semaphore::Semaphore>::acquire`

Item 8, largest functions:

- 172.8: `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0`
- 131.0: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 111.9: `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv`
- 110.1: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 84.8: `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv`
- 77.5: `<tokio::time::interval::Interval>::poll_tick`
- 29.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,578.6 | 892.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,572.3 | 1,014.3 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 826.1 | 300.7 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 636.8: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 453.0: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 186.1: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 125.2: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 52.0: `<tokio::sync::batch_semaphore::Semaphore>::try_acquire`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 40.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 35.1: `<tokio::sync::batch_semaphore::Semaphore>::acquire`

Item 6, largest functions:

- 831.7: `<std::sys::pal::unix::time::Timespec>::now`
- 357.8: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 173.1: `clock_gettime@@GLIBC_2.17`
- 135.3: `<std::time::Instant>::duration_since`
- 40.0: `<std::time::Instant>::elapsed`
- 30.5: `<std::time::Instant>::now`
- 4.0: `<tokio::time::instant::Instant>::now`
- 0.0: `<std::time::Instant>::checked_add`

Item 8, largest functions:

- 172.8: `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0`
- 131.0: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 111.6: `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv`
- 110.0: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.6: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 84.7: `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv`
- 77.4: `<tokio::time::interval::Interval>::poll_tick`
- 25.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,712.9 | 1,506.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,741.1 | 1,117.9 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 830.9 | 300.7 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,048.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 808.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 225.1: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 186.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 151.1: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 89.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 63.0: `<tokio::sync::batch_semaphore::Semaphore>::acquire`

Item 6, largest functions:

- 891.5: `<std::sys::pal::unix::time::Timespec>::now`
- 384.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 190.1: `clock_gettime@@GLIBC_2.17`
- 171.2: `<std::time::Instant>::duration_since`
- 66.0: `<std::time::Instant>::elapsed`
- 32.0: `<std::time::Instant>::now`
- 6.0: `<tokio::time::instant::Instant>::now`
- 0.0: `<std::time::Instant>::checked_add`

Item 8, largest functions:

- 172.8: `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0`
- 131.0: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 111.9: `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv`
- 110.1: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 84.8: `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv`
- 77.5: `<tokio::time::interval::Interval>::poll_tick`
- 29.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,713.3 | 1,506.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,683.5 | 1,078.7 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 827.8 | 300.7 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,048.4: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 808.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 225.1: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 186.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 151.2: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 89.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 63.0: `<tokio::sync::batch_semaphore::Semaphore>::acquire`

Item 6, largest functions:

- 918.1: `<std::sys::pal::unix::time::Timespec>::now`
- 357.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 192.8: `clock_gettime@@GLIBC_2.17`
- 135.1: `<std::time::Instant>::duration_since`
- 39.8: `<std::time::Instant>::elapsed`
- 34.4: `<std::time::Instant>::now`
- 6.0: `<tokio::time::instant::Instant>::now`
- 0.0: `<std::time::Instant>::checked_add`

Item 8, largest functions:

- 172.7: `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0`
- 131.0: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 112.2: `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv`
- 110.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 84.9: `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv`
- 77.4: `<tokio::time::interval::Interval>::poll_tick`
- 25.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

