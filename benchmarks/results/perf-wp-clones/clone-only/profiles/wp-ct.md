# wp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,764.0 | 1,764.0 | -0.0 | 26.00 | `_int_free` |
| 1,496.0 | 975.0 | -521.0 | 12.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 1,491.0 | 1,571.0 | 80.0 | 42.00 | `__GI_memcpy` |
| 1,241.0 | 1,241.0 | -0.0 | 23.00 | `malloc` |
| 1,210.0 | 1,210.0 | -0.0 | 26.00 | `free` |
| 1,139.3 | 1,139.3 | 0.0 | 11.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 1,025.2 | 1,025.7 | 0.4 | 17.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.3 | 865.3 | -0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.3 | 745.3 | -0.0 | 9.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 664.0 | 664.0 | -0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 602.4 | 602.6 | 0.2 | 10.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | 0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 522.0 | 522.0 | 0.0 | 3.00 | `_int_malloc` |
| 512.0 | 512.0 | 0.1 | 102.39 | `__aarch64_ldadd8_relax` |
| 500.5 | 503.2 | 2.7 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 484.0 | 484.0 | 0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.2 | 468.7 | 0.5 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 384.3 | 384.3 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 372.6 | 373.4 | 0.9 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 364.3 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 348.8 | 348.8 | 0.0 | 69.75 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 337.9 | 338.0 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | 0.0 | 63.75 | `__aarch64_swp4_rel` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.7 | 284.7 | -0.0 | 7.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 274.0 | 274.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 263.0 | 263.0 | 0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 255.2 | 255.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 253.4 | 0.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 248.0 | 248.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 240.0 | 240.0 | 0.0 | 2.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 240.0 | 240.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 239.0 | 239.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 237.0 | 237.0 | 0.0 | 2.00 | `malloc_consolidate` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 235.3 | 235.4 | 0.0 | 47.07 | `__aarch64_cas8_acq_rel` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 212.3 | 212.3 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 194.4 | 194.4 | 0.0 | 8.01 | `counts::main` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,496.0 | 975.0 | -521.0 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 1,491.0 | 1,571.0 | 80.0 | `__GI_memcpy` |
| 26.0 | 98.0 | 72.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 108.0 | 162.0 | 54.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 0.0 | 37.0 | 37.0 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |
| 185.0 | 181.2 | -3.8 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 500.5 | 503.2 | 2.7 | `<tokio::runtime::time::Driver>::park_internal` |
| 22.0 | 21.0 | -1.0 | `<bacnet_server::life_safety_cov::LifeSafetyCovSnapshots>::capture_write_property` |
| 372.6 | 373.4 | 0.9 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 468.2 | 468.7 | 0.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 100.9 | 101.4 | 0.5 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 1,025.2 | 1,025.7 | 0.4 | `<std::sys::pal::unix::time::Timespec>::now` |
| 136.8 | 137.2 | 0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 82.7 | 83.1 | 0.4 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 110.1 | 110.4 | 0.3 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 95.6 | 95.8 | 0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 602.4 | 602.6 | 0.2 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 164.0 | 164.1 | 0.1 | `tokio::runtime::task::waker::clone_waker` |
| 34.6 | 34.7 | 0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 107.9 | 108.0 | 0.1 | `tokio::runtime::task::waker::drop_waker` |
| 93.1 | 93.0 | -0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 51.3 | 51.4 | 0.1 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 512.0 | 512.0 | 0.1 | `__aarch64_ldadd8_relax` |
| 364.2 | 364.3 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 44.0 | 44.0 | 0.1 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 83.6 | 83.6 | 0.1 | `__aarch64_ldadd8_acq_rel` |
| 59.9 | 59.9 | 0.0 | `__aarch64_cas8_acq` |
| 235.3 | 235.4 | 0.0 | `__aarch64_cas8_acq_rel` |
| 122.7 | 122.7 | 0.0 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 348.8 | 348.8 | 0.0 | `__aarch64_cas4_acq` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 957.9 | 951.0 | -7.0 | `library/core/src/ptr/mod.rs:847` |
| 743.7 | 743.8 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 709.0 | 817.0 | 108.0 | `library/core/src/ptr/mod.rs:573` |
| 523.4 | 523.6 | 0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 429.1 | 429.2 | 0.1 | `library/core/src/sync/atomic.rs:4029` |
| 377.8 | 377.9 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 332.0 | 222.0 | -110.0 | `library/core/src/slice/iter/macros.rs:180` |
| 312.0 | 312.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 306.2 | 306.2 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 288.8 | 288.8 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.0 | 283.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 270.0 | 270.1 | 0.0 | `library/core/src/cell.rs:558` |
| 253.1 | 253.1 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 252.1 | 252.1 | 0.0 | `library/core/src/option.rs:743` |
| 247.0 | 247.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 224.0 | 224.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 190.2 | 190.3 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 185.9 | 185.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 182.0 | 112.0 | -70.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 165.3 | 165.3 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 160.0 | 160.0 | 0.0 | `library/core/src/result.rs:0` |
| 160.0 | 160.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 158.9 | 159.0 | 0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 158.0 | 128.0 | -30.0 | `library/core/src/ptr/mod.rs:1963` |
| 156.0 | 156.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 155.3 | 155.3 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 151.0 | 151.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 149.5 | 149.5 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 148.1 | 148.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 134.9 | 134.9 | 0.0 | `library/core/src/cell.rs:2437` |
| 130.9 | 130.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 130.0 | 130.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 129.0 | 129.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 128.0 | 128.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 125.0 | 125.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.0 | 121.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 120.5 | 120.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 120.0 | 0.0 | `library/core/src/slice/iter/macros.rs:379` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 118.0 | 118.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 115.0 | 115.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/arena.c:153` |
| 109.0 | 109.0 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 102.0 | 102.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 99.8 | 99.9 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
