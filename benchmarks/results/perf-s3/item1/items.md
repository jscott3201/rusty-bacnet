# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,551.0 | 847.5 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,488.7 | 781.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 759.7 | 258.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 996.7: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.1: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 670.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::release`

Item 8, largest functions:

- 364.3: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.1: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.4: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,489.0 | 781.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,443.4 | 832.8 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 737.0 | 258.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 670.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::release`

Item 6, largest functions:

- 913.9: `<std::sys::pal::unix::time::Timespec>::now`
- 356.9: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.6: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.4: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.8 | 848.1 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,430.6 | 795.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 760.3 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 998.1: `<std::sys::pal::unix::time::Timespec>::now`
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
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.7: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,450.2 | 836.1 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,432.6 | 796.7 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 737.0 | 258.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 918.7: `<std::sys::pal::unix::time::Timespec>::now`
- 358.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 173.2: `clock_gettime@@GLIBC_2.17`

Item 2a, largest functions:

- 648.1: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 410.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.1: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 1.1: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::close`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.8: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,572.2 | 1,362.6 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,645.3 | 907.8 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 760.5 | 258.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,106.4: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.4: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 195.4: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`

Item 6, largest functions:

- 1,070.8: `<std::sys::pal::unix::time::Timespec>::now`
- 384.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 190.1: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.7: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,572.7 | 1,362.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,539.3 | 893.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 737.3 | 258.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,106.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 263.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 195.5: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>::read::{closure#0}`
- 90.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckR`

Item 6, largest functions:

- 989.3: `<std::sys::pal::unix::time::Timespec>::now`
- 357.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 192.7: `clock_gettime@@GLIBC_2.17`

Item 8, largest functions:

- 364.3: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.6: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`

