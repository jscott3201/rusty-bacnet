# #1978 items by self instructions per request (variant A)

## rp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,255.0 | 665.4 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 793.7 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 468.6 | 227.3 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 804.8: `<std::sys::pal::unix::time::Timespec>::now`
- 320.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 130.1: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.4: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.0: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 92.9: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

Item 2a, largest functions:

- 281.2: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 81.2: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 10.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.2: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_server::cov::CovSubscriptionTable>>>::drop_slow`

## rp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,149.1 | 650.9 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.8 | 272.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 469.0 | 227.5 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 723.5: `<std::sys::pal::unix::time::Timespec>::now`
- 293.0: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 132.6: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.5: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.5: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 93.1: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.7: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

Item 2a, largest functions:

- 281.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 96.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 81.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 10.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<tokio::sync::batch_semaphore::Semaphore>::close`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

## rpm / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,256.5 | 666.0 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 794.5 | 272.3 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 410.7 | 245.8 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 805.9: `<std::sys::pal::unix::time::Timespec>::now`
- 320.5: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 130.2: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.5: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.5: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.0: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

Item 2a, largest functions:

- 259.5: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 81.5: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 35.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 10.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.6: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

## rpm / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 6 | 1,152.1 | 652.6 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.4 | 272.4 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |
| 2a | 411.1 | 246.0 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |

Item 6, largest functions:

- 725.5: `<std::sys::pal::unix::time::Timespec>::now`
- 293.7: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 132.9: `clock_gettime@@GLIBC_2.17`
- 0.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.5: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.4: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.8: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.6: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

Item 2a, largest functions:

- 259.7: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 81.6: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 35.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 24.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 10.1: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.7: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

## wp / ct

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,607.9 | 811.4 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,398.0 | 756.5 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 794.4 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 758.3: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 417.3: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 212.3: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 98.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

Item 6, largest functions:

- 893.7: `<std::sys::pal::unix::time::Timespec>::now`
- 352.3: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 150.1: `clock_gettime@@GLIBC_2.17`
- 2.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.7: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.3: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 108.9: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 93.1: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 76.5: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options`

## wp / mt

| Item | By function | By source file | What |
| --- | --- | --- | --- |
| 2a | 1,608.3 | 811.7 | Tokio lock acquire and release (Mutex, RwLock, Semaphore) |
| 6 | 1,293.3 | 742.5 | Clock reads (Instant::now, Timespec arithmetic, clock_gettime) |
| 8 | 771.1 | 272.2 | Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and network-control channels; the last is shared with the server's loop) |

Item 2a, largest functions:

- 758.4: `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked`
- 417.4: `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll`
- 212.4: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}`
- 98.0: `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::read::{closure#0}`
- 72.0: `<tokio::sync::batch_semaphore::Semaphore>::release`
- 50.0: `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop`
- 0.0: `<alloc::sync::Arc<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>>::drop_slow`
- 0.0: `<alloc::sync::Arc<tokio::sync::mutex::Mutex<()>>>::drop_slow`

Item 6, largest functions:

- 813.3: `<std::sys::pal::unix::time::Timespec>::now`
- 325.2: `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- 152.7: `clock_gettime@@GLIBC_2.17`
- 2.0: `<std::time::Instant>::now`

Item 8, largest functions:

- 398.3: `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}`
- 117.7: `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop`
- 92.7: `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop`
- 86.0: `tokio::runtime::task::raw::poll::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 76.4: `<tokio::time::interval::Interval>::poll_tick`
- 0.0: `<tokio::runtime::task::core::Core<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closur`
- 0.0: `tokio::runtime::task::raw::try_read_output::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#`
- 0.0: `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{cl`

