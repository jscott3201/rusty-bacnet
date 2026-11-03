# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,550.5 | 847.4 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,527.6 | 781.3 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 759.9 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 996.2: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.1: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 686.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.2: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::discovery::DeviceTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.5: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.8: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,528.1 | 781.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,443.8 | 832.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 736.9 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 686.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::discovery::DeviceTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

Item 6, largest functions:

- 914.4: `<std::sys::pal::unix::time::Timespec>::now`
- 356.9: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.5: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.3: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.8 | 848.3 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,469.9 | 795.9 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 761.2 | 258.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 998.1: `<std::sys::pal::unix::time::Timespec>::now`
- 384.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.2: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 663.6: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.6: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.7: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::discovery::DeviceTable>>>::drop_slow`

Item 8, largest functions:

- 364.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.9: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.4: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,470.2 | 796.0 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,446.7 | 834.0 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 737.3 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 663.7: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.8: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.1: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::discovery::DeviceTable>>>::drop_slow`

Item 6, largest functions:

- 916.6: `<std::sys::pal::unix::time::Timespec>::now`
- 357.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.8: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.3: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,610.9 | 1,362.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,645.0 | 907.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 760.7 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,122.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 195.3: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::discovery::DeviceTable>>>::drop_slow`

Item 6, largest functions:

- 1,070.7: `<std::sys::pal::unix::time::Timespec>::now`
- 384.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 190.1: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.7: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.3: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,611.9 | 1,362.9 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,539.8 | 894.0 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 736.9 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,122.6: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.6: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 195.6: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 90.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::discovery::DeviceTable>>>::drop_slow`

Item 6, largest functions:

- 989.5: `<std::sys::pal::unix::time::Timespec>::now`
- 357.4: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 192.8: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.1: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.5: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.8: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

