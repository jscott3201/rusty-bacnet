# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.0 | 847.5 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 794.7 | 272.1 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 468.8 | 229.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 997.7: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.6: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.8: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.3: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`

Item 2a, largest functions:

- 281.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 81.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 10.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>::lock::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::dcc_timer::TimerSlot>>>::drop_slow`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,443.9 | 832.7 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.5 | 272.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 468.8 | 229.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 914.5: `<std::sys::pal::unix::time::Timespec>::now`
- 356.9: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.5: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.5: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.1: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`

Item 2a, largest functions:

- 281.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 81.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 10.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>::lock::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::dcc_timer::TimerSlot>>>::drop_slow`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,552.4 | 848.1 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 794.1 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 410.6 | 243.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 997.7: `<std::sys::pal::unix::time::Timespec>::now`
- 384.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 170.2: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.6: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.2: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.4: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`

Item 2a, largest functions:

- 258.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 81.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 10.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>::lock::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>>::drop_slow`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,447.6 | 834.4 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.9 | 272.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 411.0 | 243.9 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 917.1: `<std::sys::pal::unix::time::Timespec>::now`
- 357.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 172.9: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.6: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.8: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

Item 2a, largest functions:

- 258.6: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 81.6: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 36.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 10.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.7: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>::lock::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>>::drop_slow`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,601.8 | 810.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,589.3 | 877.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 794.8 | 272.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 750.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 417.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 212.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.0: `<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>::lock::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>>::drop_slow`

Item 6, largest functions:

- 1,025.0: `<std::sys::pal::unix::time::Timespec>::now`
- 384.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 180.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.5: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.6: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}>`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,602.1 | 810.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,483.5 | 863.2 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.6 | 272.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 750.4: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 417.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 212.4: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 100.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::close`
- 0.0: `<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>::lock::{closure#0}`

Item 6, largest functions:

- 943.8: `<std::sys::pal::unix::time::Timespec>::now`
- 357.1: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 182.6: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.6: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.1: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `<tokio::runtime::task::harness::Harness<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

