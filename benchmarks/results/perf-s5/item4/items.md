# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.1 | 847.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,527.9 | 779.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 759.6 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 997.8: `<std::sys::pal::unix::time::Timespec>::now`
- 384.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 2a, largest functions:

- 686.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::tsm::Tsm>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`

Item 8, largest functions:

- 364.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 116.9: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,529.5 | 780.2 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,448.7 | 835.3 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 736.7 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 686.8: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.8: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.8: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::close`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::tsm::Tsm>>>::drop_slow`

Item 6, largest functions:

- 917.7: `<std::sys::pal::unix::time::Timespec>::now`
- 358.0: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 173.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 364.1: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.7: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.5 | 848.1 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 2a | 1,469.6 | 795.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 8 | 759.8 | 258.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 6, largest functions:

- 997.8: `<std::sys::pal::unix::time::Timespec>::now`
- 384.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.2: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 2a, largest functions:

- 663.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::tsm::Tsm>>>::drop_slow`

Item 8, largest functions:

- 364.2: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.8: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,469.9 | 795.9 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,446.7 | 834.0 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 736.8 | 258.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 663.6: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 409.6: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 50.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::tsm::Tsm>>>::drop_slow`

Item 6, largest functions:

- 916.5: `<std::sys::pal::unix::time::Timespec>::now`
- 357.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.8: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 364.1: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.8: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,661.1 | 1,362.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,590.4 | 877.8 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 760.7 | 258.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,155.4: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.0: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 212.4: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 90.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::tsm::Tsm>>>::drop_slow`

Item 6, largest functions:

- 1,026.0: `<std::sys::pal::unix::time::Timespec>::now`
- 384.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 180.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 364.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.7: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.1: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 2,662.6 | 1,363.3 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,487.1 | 865.4 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 737.4 | 258.5 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 1,155.8: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 745.9: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 286.1: `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{c`
- 212.7: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 90.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_client::tsm::Tsm>>>::drop_slow`

Item 6, largest functions:

- 946.1: `<std::sys::pal::unix::time::Timespec>::now`
- 358.0: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 183.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 364.6: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 116.9: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.8: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

