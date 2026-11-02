# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,652.8 | 874.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,627.9 | 1,051.8 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 830.6 | 300.7 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 698.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 506.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 186.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 125.1: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 52.0: `<tokio::sync::batch_semaphore::Semaphore>::try_acquire`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 35.0: `<tokio::sync::batch_semaphore::Semaphore>::acquire`
- 0.1: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`

Item 6, largest functions:

- 804.4: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 171.1: `<std::time::Instant>::duration_since`
- 170.1: `clock_gettime@@GLIBC_2.17`
- 66.0: `<std::time::Instant>::elapsed`
- 28.0: `<std::time::Instant>::now`
- 4.0: `<tokio::time::instant::Instant>::now`
- 0.0: `<std::time::Instant>::checked_add`

Item 8, largest functions:

- 172.7: `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0`
- 131.0: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 111.9: `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv`
- 109.9: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 84.7: `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv`
- 77.5: `<tokio::time::interval::Interval>::poll_tick`
- 29.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,653.2 | 874.7 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,569.5 | 1,012.2 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 828.0 | 300.8 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 698.4: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 506.4: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 186.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 125.1: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 52.0: `<tokio::sync::batch_semaphore::Semaphore>::try_acquire`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 35.0: `<tokio::sync::batch_semaphore::Semaphore>::acquire`
- 0.2: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`

Item 6, largest functions:

- 830.4: `<std::sys::pal::unix::time::Timespec>::now`
- 357.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.7: `clock_gettime@@GLIBC_2.17`
- 135.0: `<std::time::Instant>::duration_since`
- 39.7: `<std::time::Instant>::elapsed`
- 30.4: `<std::time::Instant>::now`
- 4.0: `<tokio::time::instant::Instant>::now`
- 0.0: `<std::time::Instant>::checked_add`

Item 8, largest functions:

- 172.7: `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0`
- 131.0: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 112.5: `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv`
- 110.0: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.4: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 84.8: `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv`
- 77.6: `<tokio::time::interval::Interval>::poll_tick`
- 25.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

