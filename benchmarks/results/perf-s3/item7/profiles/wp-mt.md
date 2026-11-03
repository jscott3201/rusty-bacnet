# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,750.3 | 1,750.1 | -0.1 | 26.00 | `_int_free` |
| 1,603.0 | 1,603.0 | 0.0 | 49.00 | `__GI_memcpy` |
| 1,270.8 | 1,272.5 | 1.7 | 26.00 | `free` |
| 1,247.1 | 1,247.1 | -0.0 | 23.00 | `malloc` |
| 1,122.6 | 1,122.5 | -0.1 | 11.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 989.5 | 989.7 | 0.1 | 15.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.6 | 865.5 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.6 | 745.5 | -0.1 | 9.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 702.1 | 634.9 | -67.2 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 659.0 | 659.0 | 0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.8 | 567.7 | -0.1 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 539.4 | 539.4 | -0.1 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 521.0 | 521.0 | 0.0 | 4.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 512.0 | 512.1 | 0.1 | 102.39 | `__aarch64_ldadd8_relax` |
| 505.4 | 505.3 | -0.1 | 97.05 | `__aarch64_cas4_acq` |
| 484.1 | 484.5 | 0.3 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.5 | 471.3 | 2.7 | 3.14 | `_int_malloc` |
| 467.8 | 468.5 | 0.8 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 426.1 | 426.1 | 0.0 | 1.32 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 379.6 | 379.7 | 0.1 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.8 | 378.7 | -0.1 | 74.95 | `__aarch64_swp4_rel` |
| 373.8 | 374.4 | 0.6 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.1 | 364.3 | 0.2 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 357.4 | 357.3 | -0.1 | 11.17 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.2 | 338.2 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.2 | 319.2 | 0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 286.0 | 286.0 | -0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 286.0 | 286.0 | -0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.7 | 280.6 | -0.1 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 270.4 | 270.4 | 0.0 | 54.08 | `__aarch64_cas8_acq_rel` |
| 267.9 | 267.9 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 253.4 | 0.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 243.4 | 243.5 | 0.0 | 6.32 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 240.0 | 240.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 239.0 | 240.0 | 1.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 211.9 | 211.8 | -0.0 | 2.07 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.2 | 204.2 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 197.6 | 197.4 | -0.2 | 4.02 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 702.1 | 634.9 | -67.2 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 51.8 | 31.8 | -20.1 | `_int_free_maybe_consolidate.part.0` |
| 42.0 | 28.0 | -14.0 | `core::ptr::drop_glue::<bacnet_network::response_route::ResponseRoute>` |
| 21.3 | 27.1 | 5.7 | `_int_free_create_chunk` |
| 143.9 | 140.0 | -3.9 | `malloc_consolidate` |
| 468.5 | 471.3 | 2.7 | `_int_malloc` |
| 6.5 | 4.1 | -2.4 | `alloc_perturb` |
| 7.1 | 9.1 | 2.0 | `unlink_chunk.isra.0` |
| 1,270.8 | 1,272.5 | 1.7 | `free` |
| 239.0 | 240.0 | 1.0 | `bacnet_encoding::apdu::decode_apdu` |
| 467.8 | 468.5 | 0.8 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 373.8 | 374.4 | 0.6 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 49.6 | 49.3 | -0.4 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 484.1 | 484.5 | 0.3 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 41.9 | 41.6 | -0.3 | `_int_free_merge_chunk` |
| 101.4 | 101.7 | 0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 107.9 | 108.1 | 0.3 | `tokio::runtime::task::waker::drop_waker` |
| 364.1 | 364.3 | 0.2 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 197.6 | 197.4 | -0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 92.8 | 93.0 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 164.0 | 164.3 | 0.2 | `tokio::runtime::task::waker::clone_waker` |
| 83.0 | 83.2 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 1.2 | 1.1 | -0.2 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 90.7 | 90.6 | -0.2 | `<tokio::runtime::time::handle::Handle>::reregister` |
| 126.6 | 126.8 | 0.2 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 0.9 | 0.8 | -0.1 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 1,750.3 | 1,750.1 | -0.1 | `_int_free` |
| 989.5 | 989.7 | 0.1 | `<std::sys::pal::unix::time::Timespec>::now` |
| 137.0 | 136.9 | -0.1 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 357.4 | 357.3 | -0.1 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 951.5 | 929.4 | -22.1 | `library/core/src/ptr/mod.rs:847` |
| 846.0 | 909.0 | 63.0 | `library/core/src/ptr/mod.rs:573` |
| 771.7 | 771.8 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 588.5 | 588.5 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 426.2 | 426.2 | 0.1 | `library/core/src/sync/atomic.rs:4029` |
| 399.9 | 399.8 | -0.1 | `library/core/src/sync/atomic.rs:3994` |
| 327.8 | 327.7 | -0.1 | `library/std/src/sys/pal/unix/time.rs:107` |
| 314.6 | 314.5 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 312.0 | 312.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 307.8 | 307.7 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 271.8 | 271.7 | -0.1 | `library/std/src/panicking.rs:465` |
| 255.3 | 255.3 | 0.0 | `library/core/src/cell.rs:558` |
| 249.5 | 249.5 | -0.0 | `library/core/src/option.rs:743` |
| 233.6 | 231.6 | -2.0 | `library/core/src/mem/mod.rs:976` |
| 224.0 | 224.0 | 0.0 | `library/core/src/result.rs:2177` |
| 222.7 | 222.6 | -0.0 | `???:0` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 192.9 | 192.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 190.3 | 190.3 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 168.7 | 168.7 | -0.1 | `library/core/src/sync/atomic.rs:4013` |
| 167.0 | 167.0 | 0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 166.4 | 166.4 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 163.5 | 163.4 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 160.0 | 160.0 | 0.0 | `library/core/src/result.rs:0` |
| 160.0 | 160.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 157.0 | 157.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 156.0 | 156.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 151.5 | 151.4 | -0.1 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 150.7 | 150.8 | 0.0 | `library/core/src/cell.rs:2437` |
| 148.1 | 148.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 147.1 | 147.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.9 | 138.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 129.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 125.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.1 | 121.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 120.0 | 120.0 | 0.0 | `library/core/src/slice/iter/macros.rs:379` |
| 118.0 | 118.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 108.1 | 108.2 | 0.1 | `library/core/src/ptr/mod.rs:1963` |
| 107.2 | 107.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.9 | 105.5 | -0.3 | `./malloc/./malloc/arena.c:147` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 99.7 | 99.9 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
