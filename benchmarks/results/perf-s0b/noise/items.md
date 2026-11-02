# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,550.9 | 847.4 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,488.6 | 781.3 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 760.0 | 258.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 996.6: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.1: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 670.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.2: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.2: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,488.9 | 781.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,440.7 | 832.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 738.1 | 258.5 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 670.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`

Item 6, largest functions:

- 911.3: `<std::sys::pal::unix::time::Timespec>::now`
- 356.9: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.5: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.8: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.9: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.4 | 848.1 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,430.7 | 795.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 760.1 | 258.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 997.8: `<std::sys::pal::unix::time::Timespec>::now`
- 384.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.2: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 647.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,446.8 | 834.0 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,431.3 | 796.1 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 736.2 | 258.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 916.6: `<std::sys::pal::unix::time::Timespec>::now`
- 357.4: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.8: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 647.8: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.8: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::close`

Item 8, largest functions:

- 364.1: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.2: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.6: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.3: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,571.8 | 1,362.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,643.8 | 907.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 759.8 | 258.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,106.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 195.3: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

Item 6, largest functions:

- 1,069.5: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 190.1: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.3: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,572.1 | 1,362.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,538.1 | 893.0 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 736.5 | 258.1 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,106.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 195.4: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`

Item 6, largest functions:

- 988.5: `<std::sys::pal::unix::time::Timespec>::now`
- 357.0: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 192.6: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.1: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.7: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.3: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

