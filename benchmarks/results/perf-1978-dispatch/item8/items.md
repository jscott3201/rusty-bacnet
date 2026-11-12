# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,254.7 | 665.4 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 795.2 | 272.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 468.6 | 227.3 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 804.5: `<std::sys::pal::unix::time::Timespec>::now`
- 320.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 130.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.7: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.7: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.3: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

Item 2a, largest functions:

- 281.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 81.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 10.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.2: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::server::device_bindings::DeviceBindingTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,144.5 | 650.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.7 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 468.8 | 227.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 719.1: `<std::sys::pal::unix::time::Timespec>::now`
- 292.9: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 132.5: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.5: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.5: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.2: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.4: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

Item 2a, largest functions:

- 281.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 81.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 10.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::close`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::server::device_bindings::DeviceBindingTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,256.1 | 665.9 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 793.8 | 272.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 407.5 | 239.7 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 805.5: `<std::sys::pal::unix::time::Timespec>::now`
- 320.4: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 130.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.7: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

Item 2a, largest functions:

- 257.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 81.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 34.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 10.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.5: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::server::device_bindings::DeviceBindingTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,150.8 | 651.9 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.3 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 407.7 | 239.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 724.7: `<std::sys::pal::unix::time::Timespec>::now`
- 293.4: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 132.8: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.6: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.0: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.1: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

Item 2a, largest functions:

- 257.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 81.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 34.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 10.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::server::device_bindings::DeviceBindingTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,593.8 | 802.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,398.8 | 756.5 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 793.2 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 744.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 417.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 212.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 98.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::server::device_bindings::DeviceBindingTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

Item 6, largest functions:

- 894.5: `<std::sys::pal::unix::time::Timespec>::now`
- 352.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 150.1: `clock_gettime@@GLIBC_2.17`
- 2.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.0: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.6: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.3: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,593.9 | 802.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,292.7 | 741.7 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.0 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 744.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 417.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 212.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 98.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::server::device_bindings::DeviceBindingTable>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<bacnet_server::server::learned_router_cache::LearnedRouterCache>>>::drop_slow`

Item 6, largest functions:

- 813.2: `<std::sys::pal::unix::time::Timespec>::now`
- 324.9: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 152.5: `clock_gettime@@GLIBC_2.17`
- 2.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.4: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

