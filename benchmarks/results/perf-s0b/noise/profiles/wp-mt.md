# wp / mt: self instructions per request

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,749.9 | 1,750.2 | 0.3 | 26.00 | `_int_free` |
| 1,496.0 | 1,496.0 | 0.0 | 12.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 1,491.0 | 1,491.0 | 0.0 | 42.00 | `__GI_memcpy` |
| 1,271.7 | 1,272.5 | 0.8 | 26.00 | `free` |
| 1,247.1 | 1,247.1 | 0.0 | 23.00 | `malloc` |
| 1,106.3 | 1,106.3 | -0.0 | 11.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 988.5 | 988.5 | 0.0 | 15.14 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.4 | 0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.3 | 745.3 | 0.0 | 9.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 659.0 | 659.0 | 0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 602.9 | 602.8 | -0.1 | 10.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | -0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 511.9 | 512.0 | 0.1 | 102.38 | `__aarch64_ldadd8_relax` |
| 505.1 | 505.1 | -0.0 | 97.00 | `__aarch64_cas4_acq` |
| 484.3 | 484.5 | 0.1 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 467.7 | 467.8 | 0.1 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 463.8 | 476.0 | 12.2 | 3.31 | `_int_malloc` |
| 424.7 | 425.7 | 1.0 | 1.33 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 378.5 | 378.5 | -0.0 | 74.91 | `__aarch64_swp4_rel` |
| 378.4 | 379.0 | 0.6 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 373.8 | 373.6 | -0.2 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.1 | 364.2 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 357.0 | 357.0 | -0.0 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.1 | 338.0 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.1 | 319.1 | 0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.4 | 280.4 | -0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 270.2 | 270.2 | 0.0 | 54.04 | `__aarch64_cas8_acq_rel` |
| 263.0 | 263.0 | 0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 253.4 | -0.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 247.9 | 247.9 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 243.6 | 243.6 | 0.0 | 6.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 240.0 | 240.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 239.0 | 239.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 233.0 | 233.0 | 0.0 | 2.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 211.6 | 211.1 | -0.5 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.1 | 204.2 | 0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 197.2 | 197.2 | -0.0 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 463.8 | 476.0 | 12.2 | `_int_malloc` |
| 41.2 | 32.9 | -8.3 | `_int_free_maybe_consolidate.part.0` |
| 24.4 | 26.8 | 2.4 | `_int_free_create_chunk` |
| 127.2 | 125.4 | -1.8 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 8.8 | 10.1 | 1.3 | `unlink_chunk.isra.0` |
| 5.8 | 4.7 | -1.1 | `alloc_perturb` |
| 424.7 | 425.7 | 1.0 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 141.3 | 140.5 | -0.9 | `malloc_consolidate` |
| 1,271.7 | 1,272.5 | 0.8 | `free` |
| 378.4 | 379.0 | 0.6 | `<tokio::runtime::time::Driver>::park_internal` |
| 211.6 | 211.1 | -0.5 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 92.7 | 93.0 | 0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 1,749.9 | 1,750.2 | 0.3 | `_int_free` |
| 87.3 | 87.5 | 0.2 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 100.8 | 101.0 | 0.2 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 107.8 | 108.0 | 0.2 | `tokio::runtime::task::waker::drop_waker` |
| 163.8 | 164.1 | 0.2 | `tokio::runtime::task::waker::clone_waker` |
| 136.8 | 137.0 | 0.2 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 373.8 | 373.6 | -0.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 117.3 | 117.5 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 484.3 | 484.5 | 0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 82.7 | 82.8 | 0.1 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 41.7 | 41.6 | -0.1 | `_int_free_merge_chunk` |
| 76.3 | 76.5 | 0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 602.9 | 602.8 | -0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 95.5 | 95.7 | 0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 467.7 | 467.8 | 0.1 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 113.3 | 113.3 | 0.1 | `__aarch64_ldadd8_acq_rel` |
| 364.1 | 364.2 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 511.9 | 512.0 | 0.1 | `__aarch64_ldadd8_relax` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 951.3 | 951.3 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 771.5 | 771.5 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 709.0 | 709.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 588.1 | 588.2 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 430.1 | 430.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 401.6 | 401.6 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 334.0 | 334.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 327.4 | 327.4 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 314.4 | 314.4 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 312.0 | 312.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 307.6 | 307.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 271.6 | 271.6 | -0.0 | `library/std/src/panicking.rs:465` |
| 255.2 | 255.2 | 0.0 | `library/core/src/cell.rs:558` |
| 249.4 | 249.4 | -0.0 | `library/core/src/option.rs:743` |
| 233.5 | 233.5 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 224.0 | 224.0 | 0.0 | `library/core/src/result.rs:2177` |
| 222.6 | 222.6 | 0.0 | `???:0` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 192.9 | 192.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 190.2 | 190.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 182.0 | 182.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 168.6 | 168.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 166.9 | 166.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 166.3 | 166.3 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 163.3 | 163.3 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 160.0 | 160.0 | 0.0 | `library/core/src/result.rs:0` |
| 160.0 | 160.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.0 | 156.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 155.0 | 155.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 151.7 | 151.7 | 0.0 | `library/core/src/cell.rs:2437` |
| 151.3 | 151.3 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 148.1 | 148.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 147.0 | 147.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.8 | 138.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 129.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 125.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.0 | 121.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 120.0 | 120.0 | 0.0 | `library/core/src/slice/iter/macros.rs:379` |
| 118.0 | 118.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 107.1 | 107.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.7 | 105.6 | -0.1 | `./malloc/./malloc/arena.c:147` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 99.7 | 99.7 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
