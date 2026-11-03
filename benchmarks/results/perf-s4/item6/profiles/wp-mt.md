# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,750.2 | 1,750.6 | 0.4 | 26.00 | `_int_free` |
| 1,651.0 | 1,651.0 | 0.0 | 52.00 | `__GI_memcpy` |
| 1,247.1 | 1,247.1 | 0.0 | 23.00 | `malloc` |
| 1,239.6 | 1,238.8 | -0.8 | 26.00 | `free` |
| 943.8 | 873.7 | -70.1 | 14.14 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.5 | 0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 750.4 | 750.3 | -0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 659.0 | 659.0 | 0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 634.9 | 698.0 | 63.1 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.6 | 567.5 | -0.1 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 539.3 | 539.3 | -0.1 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 511.9 | 512.0 | 0.1 | 102.39 | `__aarch64_ldadd8_relax` |
| 505.1 | 505.1 | 0.0 | 97.00 | `__aarch64_cas4_acq` |
| 484.5 | 484.3 | -0.2 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.2 | 468.6 | 0.4 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 463.0 | 475.6 | 12.6 | 3.51 | `_int_malloc` |
| 426.0 | 423.8 | -2.2 | 1.31 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.3 | 417.3 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.6 | 398.6 | 0.0 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 379.4 | 377.9 | -1.4 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.7 | 378.8 | 0.1 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.5 | 378.6 | 0.0 | 74.91 | `__aarch64_swp4_rel` |
| 357.1 | 357.1 | -0.0 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.1 | 338.1 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.1 | 319.1 | -0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 317.9 | 317.9 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | -0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.5 | 280.4 | -0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 250.9 | 250.9 | 0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 250.3 | 250.3 | 0.0 | 50.06 | `__aarch64_cas8_acq_rel` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 234.0 | 234.0 | -0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | -0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.4 | 212.4 | -0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 211.5 | 211.8 | 0.3 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.2 | 204.1 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 197.2 | 197.4 | 0.2 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.2 | 193.2 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 170.3 | 0.0 | -170.3 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 943.8 | 873.7 | -70.1 | `<std::sys::pal::unix::time::Timespec>::now` |
| 634.9 | 698.0 | 63.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 58.1 | 0.0 | -58.1 | `core::ptr::drop_glue::<alloc::vec::into_iter::IntoIter<bacnet_server::server::segmented_receive::ExpiredRequest>>` |
| 182.6 | 162.6 | -20.0 | `clock_gettime@@GLIBC_2.17` |
| 18.0 | 0.0 | -18.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}>` |
| 463.0 | 475.6 | 12.6 | `_int_malloc` |
| 29.2 | 39.0 | 9.8 | `_int_free_maybe_consolidate.part.0` |
| 137.9 | 143.0 | 5.1 | `malloc_consolidate` |
| 125.8 | 129.1 | 3.3 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 27.7 | 25.0 | -2.8 | `_int_free_create_chunk` |
| 426.0 | 423.8 | -2.2 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 379.4 | 377.9 | -1.4 | `<tokio::runtime::time::Driver>::park_internal` |
| 4.5 | 5.9 | 1.4 | `alloc_perturb` |
| 1,239.6 | 1,238.8 | -0.8 | `free` |
| 10.3 | 10.8 | 0.5 | `unlink_chunk.isra.0` |
| 1,750.2 | 1,750.6 | 0.4 | `_int_free` |
| 468.2 | 468.6 | 0.4 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 87.5 | 87.9 | 0.3 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 96.4 | 96.7 | 0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 211.5 | 211.8 | 0.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 117.3 | 117.5 | 0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 136.8 | 137.0 | 0.2 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 197.2 | 197.4 | 0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 0.5 | 0.6 | 0.2 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |
| 93.1 | 93.3 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 41.5 | 41.7 | 0.2 | `_int_free_merge_chunk` |
| 865.4 | 865.5 | 0.2 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 484.5 | 484.3 | -0.2 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 101.2 | 101.1 | -0.1 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 988.0 | 989.0 | 1.0 | `library/core/src/ptr/mod.rs:573` |
| 866.4 | 828.3 | -38.1 | `library/core/src/ptr/mod.rs:847` |
| 755.6 | 755.6 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 572.2 | 572.3 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 435.1 | 435.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 400.7 | 400.7 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 312.0 | 312.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 311.4 | 311.4 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 310.5 | 276.4 | -34.1 | `library/std/src/sys/pal/unix/time.rs:107` |
| 307.6 | 307.6 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 271.6 | 271.6 | 0.0 | `library/std/src/panicking.rs:465` |
| 257.4 | 257.4 | 0.0 | `library/core/src/option.rs:743` |
| 236.7 | 236.8 | 0.0 | `library/core/src/cell.rs:558` |
| 229.0 | 229.0 | 0.0 | `library/core/src/result.rs:2177` |
| 227.5 | 225.6 | -2.0 | `library/core/src/mem/mod.rs:976` |
| 222.6 | 222.6 | 0.0 | `???:0` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 188.9 | 188.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.3 | 174.3 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 165.6 | 165.6 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 164.3 | 164.3 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 162.4 | 162.4 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 158.0 | 158.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.0 | 156.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 155.0 | 155.0 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 151.4 | 151.3 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 144.7 | 144.7 | 0.0 | `library/core/src/cell.rs:2437` |
| 143.1 | 143.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.8 | 138.8 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 130.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 129.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 125.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 113.0 | 113.0 | -0.0 | `library/core/src/result.rs:0` |
| 105.5 | 105.7 | 0.2 | `./malloc/./malloc/arena.c:147` |
| 105.1 | 105.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 99.8 | 99.9 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
