# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,750.9 | 1,749.7 | -1.3 | 26.00 | `_int_free` |
| 1,496.0 | 521.0 | -975.0 | 12.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 1,491.0 | 1,603.0 | 112.0 | 42.00 | `__GI_memcpy` |
| 1,270.9 | 1,270.7 | -0.1 | 26.00 | `free` |
| 1,247.1 | 1,247.1 | -0.0 | 23.00 | `malloc` |
| 1,106.5 | 1,122.4 | 15.9 | 11.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 989.3 | 988.8 | -0.4 | 15.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.6 | 865.6 | 0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.5 | 745.4 | -0.1 | 9.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 659.0 | 659.0 | -0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 602.9 | 702.0 | 99.1 | 10.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.7 | 567.6 | -0.1 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 539.4 | 539.3 | -0.1 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 512.0 | 512.0 | -0.0 | 102.41 | `__aarch64_ldadd8_relax` |
| 505.3 | 505.2 | -0.1 | 97.03 | `__aarch64_cas4_acq` |
| 484.2 | 484.1 | -0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 474.6 | 485.8 | 11.2 | 3.16 | `_int_malloc` |
| 468.1 | 468.6 | 0.5 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 426.5 | 424.9 | -1.6 | 1.31 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 379.6 | 378.7 | -0.9 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.7 | 378.6 | -0.1 | 74.93 | `__aarch64_swp4_rel` |
| 374.0 | 374.1 | 0.1 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.3 | 364.2 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 357.2 | 357.3 | 0.1 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 338.2 | 338.1 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.1 | 319.2 | 0.1 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.6 | 280.6 | -0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 270.4 | 270.4 | -0.1 | 54.09 | `__aarch64_cas8_acq_rel` |
| 263.0 | 286.0 | 23.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 253.4 | 0.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 248.0 | 267.9 | 19.9 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 243.6 | 243.6 | -0.0 | 6.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 240.0 | 240.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 239.0 | 239.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 233.0 | 286.0 | 53.0 | 2.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 211.7 | 211.6 | -0.1 | 2.07 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.2 | 204.2 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 197.4 | 197.5 | 0.0 | 4.02 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,496.0 | 521.0 | -975.0 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 65.0 | 189.0 | 124.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 154.0 | 41.0 | -113.0 | `<bacnet_server::server::confirmed_request_tracker::ConfirmedRequestTracker>::begin` |
| 1,491.0 | 1,603.0 | 112.0 | `__GI_memcpy` |
| 602.9 | 702.0 | 99.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 0.0 | 59.0 | 59.0 | `<bacnet_endpoint_core::coordinator::CanonicalPeer as core::clone::Clone>::clone` |
| 233.0 | 286.0 | 53.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 80.0 | 124.0 | 44.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 263.0 | 286.0 | 23.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 87.0 | 109.0 | 22.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |
| 248.0 | 267.9 | 19.9 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 1,106.5 | 1,122.4 | 15.9 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 474.6 | 485.8 | 11.2 | `_int_malloc` |
| 7.6 | 15.5 | 7.9 | `unlink_chunk.isra.0` |
| 426.5 | 424.9 | -1.6 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 125.2 | 126.8 | 1.6 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 1,750.9 | 1,749.7 | -1.3 | `_int_free` |
| 117.0 | 116.0 | -1.0 | `<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>::admit` |
| 155.0 | 156.0 | 1.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 49.9 | 48.9 | -1.0 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 379.6 | 378.7 | -0.9 | `<tokio::runtime::time::Driver>::park_internal` |
| 50.7 | 51.5 | 0.8 | `_int_free_maybe_consolidate.part.0` |
| 468.1 | 468.6 | 0.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 989.3 | 988.8 | -0.4 | `<std::sys::pal::unix::time::Timespec>::now` |
| 117.6 | 118.0 | 0.4 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.5 | 87.9 | 0.4 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 144.3 | 144.0 | -0.3 | `malloc_consolidate` |
| 101.6 | 101.3 | -0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 136.3 | 136.6 | 0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 107.9 | 108.2 | 0.3 | `tokio::runtime::task::waker::drop_waker` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 951.5 | 951.4 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 771.8 | 771.7 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 709.0 | 846.0 | 137.0 | `library/core/src/ptr/mod.rs:573` |
| 588.5 | 588.4 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 430.2 | 426.2 | -4.0 | `library/core/src/sync/atomic.rs:4029` |
| 401.8 | 399.7 | -2.1 | `library/core/src/sync/atomic.rs:3994` |
| 334.0 | 157.0 | -177.0 | `library/core/src/slice/iter/macros.rs:180` |
| 327.7 | 327.7 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 314.5 | 314.5 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 312.0 | 312.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 307.7 | 307.7 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 271.7 | 271.7 | -0.0 | `library/std/src/panicking.rs:465` |
| 255.3 | 255.3 | -0.1 | `library/core/src/cell.rs:558` |
| 249.5 | 249.4 | -0.0 | `library/core/src/option.rs:743` |
| 233.7 | 233.6 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 224.0 | 224.0 | 0.0 | `library/core/src/result.rs:2177` |
| 222.7 | 222.6 | -0.1 | `???:0` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 192.9 | 192.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 190.4 | 190.3 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 182.0 | 70.0 | -112.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 168.7 | 168.7 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 167.0 | 167.0 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 166.4 | 166.4 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 163.4 | 163.4 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 160.0 | 160.0 | -0.0 | `library/core/src/result.rs:0` |
| 160.0 | 160.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.0 | 156.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 155.1 | 108.3 | -46.8 | `library/core/src/ptr/mod.rs:1963` |
| 151.8 | 150.8 | -1.0 | `library/core/src/cell.rs:2437` |
| 151.5 | 151.4 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 148.1 | 148.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 147.1 | 147.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.8 | 138.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 129.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 125.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.1 | 121.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 120.0 | 120.0 | 0.0 | `library/core/src/slice/iter/macros.rs:379` |
| 118.0 | 118.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 115.0 | 115.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 107.1 | 107.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.9 | 105.9 | 0.0 | `./malloc/./malloc/arena.c:147` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 99.8 | 99.9 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
