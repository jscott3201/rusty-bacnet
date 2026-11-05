# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,621.0 | 1,621.0 | 0.0 | 50.00 | `__GI_memcpy` |
| 1,620.1 | 1,620.2 | 0.1 | 24.00 | `_int_free` |
| 1,155.6 | 1,155.5 | -0.1 | 24.00 | `free` |
| 1,139.1 | 1,139.1 | 0.0 | 21.00 | `malloc` |
| 873.9 | 751.7 | -122.2 | 12.14 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.7 | 0.3 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 750.3 | 750.3 | 0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 697.9 | 697.8 | -0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 607.0 | 607.0 | -0.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.5 | 567.5 | 0.0 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.3 | 539.3 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 532.0 | 736.0 | 204.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 505.1 | 505.1 | -0.0 | 97.01 | `__aarch64_cas4_acq` |
| 492.0 | 491.9 | -0.1 | 98.40 | `__aarch64_ldadd8_relax` |
| 483.5 | 483.4 | -0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.6 | 468.1 | -0.5 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 454.9 | 457.5 | 2.6 | 3.46 | `_int_malloc` |
| 425.5 | 424.5 | -1.0 | 1.32 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.3 | 417.3 | 0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 398.6 | 398.6 | -0.0 | 7.76 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 379.0 | 378.2 | -0.8 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.6 | 378.2 | -0.4 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.6 | 378.5 | -0.0 | 74.91 | `__aarch64_swp4_rel` |
| 357.0 | 293.1 | -64.0 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.1 | 338.1 | 0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.1 | 319.1 | 0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 317.9 | 317.9 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.4 | 280.5 | 0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 250.9 | 250.8 | -0.0 | 5.32 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 250.3 | 250.2 | -0.0 | 50.06 | `__aarch64_cas8_acq_rel` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 234.0 | 0.0 | -234.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | -0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 229.0 | 229.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.4 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 211.7 | 211.9 | 0.2 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.1 | 204.2 | 0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 197.2 | 197.4 | 0.2 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.2 | 193.2 | 0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 234.0 | 0.0 | -234.0 | `<bacnet_time::host::HostClock>::now` |
| 532.0 | 736.0 | 204.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 873.9 | 751.7 | -122.2 | `<std::sys::pal::unix::time::Timespec>::now` |
| 357.0 | 293.1 | -64.0 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 162.6 | 142.6 | -20.0 | `clock_gettime@@GLIBC_2.17` |
| 4.0 | 0.0 | -4.0 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 32.7 | 35.3 | 2.6 | `_int_free_maybe_consolidate.part.0` |
| 454.9 | 457.5 | 2.6 | `_int_malloc` |
| 126.2 | 128.1 | 1.9 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 425.5 | 424.5 | -1.0 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 379.0 | 378.2 | -0.8 | `<tokio::runtime::time::Driver>::park_internal` |
| 26.8 | 26.0 | -0.7 | `_int_free_create_chunk` |
| 468.6 | 468.1 | -0.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 139.7 | 140.2 | 0.5 | `malloc_consolidate` |
| 0.5 | 0.9 | 0.4 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |
| 378.6 | 378.2 | -0.4 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 101.2 | 100.9 | -0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 4.7 | 5.0 | 0.3 | `alloc_perturb` |
| 136.7 | 136.4 | -0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 865.4 | 865.7 | 0.3 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 197.2 | 197.4 | 0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 96.5 | 96.2 | -0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 6.6 | 6.4 | -0.2 | `unlink_chunk.isra.0` |
| 93.3 | 93.1 | -0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 211.7 | 211.9 | 0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 83.0 | 82.8 | -0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 108.1 | 107.9 | -0.2 | `tokio::runtime::task::waker::drop_waker` |
| 164.2 | 164.1 | -0.1 | `tokio::runtime::task::waker::clone_waker` |
| 1,155.6 | 1,155.5 | -0.1 | `free` |
| 121.6 | 121.5 | -0.1 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 955.0 | 955.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 821.3 | 821.3 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 739.6 | 739.5 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 572.3 | 572.2 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 413.2 | 413.1 | -0.1 | `library/core/src/sync/atomic.rs:4029` |
| 398.7 | 398.7 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 311.4 | 311.4 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 307.6 | 307.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 288.0 | 288.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 276.4 | 242.5 | -34.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 271.6 | 271.6 | -0.0 | `library/std/src/panicking.rs:465` |
| 253.4 | 253.4 | -0.0 | `library/core/src/option.rs:743` |
| 234.7 | 234.7 | -0.1 | `library/core/src/cell.rs:558` |
| 224.5 | 224.5 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 222.6 | 222.6 | 0.0 | `???:0` |
| 217.0 | 217.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 189.0 | 189.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 184.9 | 184.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.3 | 174.2 | -0.1 | `library/core/src/sync/atomic.rs:4104` |
| 165.6 | 165.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 162.4 | 162.3 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 158.3 | 158.3 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 155.0 | 154.9 | -0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 150.3 | 150.4 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 150.0 | 150.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 144.7 | 144.7 | -0.0 | `library/core/src/cell.rs:2437` |
| 144.0 | 144.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 143.1 | 143.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 139.0 | 139.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 138.8 | 138.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 128.0 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 126.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 126.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 126.0 | 126.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 121.0 | 121.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 120.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 115.0 | 115.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 107.0 | 107.0 | 0.0 | `library/core/src/result.rs:0` |
| 105.1 | 105.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.0 | 105.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 99.9 | 99.8 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 98.0 | 98.5 | 0.5 | `./malloc/./malloc/arena.c:147` |
| 96.1 | 96.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
