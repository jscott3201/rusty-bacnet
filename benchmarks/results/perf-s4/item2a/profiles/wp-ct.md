# wp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,764.0 | 1,764.0 | 0.0 | 26.00 | `_int_free` |
| 1,651.0 | 1,651.0 | 0.0 | 52.00 | `__GI_memcpy` |
| 1,241.0 | 1,241.0 | 0.0 | 23.00 | `malloc` |
| 1,210.0 | 1,177.0 | -33.0 | 26.00 | `free` |
| 1,155.3 | 750.3 | -405.0 | 11.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 1,025.6 | 1,025.4 | -0.2 | 17.64 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.5 | 865.4 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.3 | 417.3 | -328.0 | 9.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 664.0 | 664.0 | 0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 634.6 | 634.6 | -0.0 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.5 | 567.5 | 0.0 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 539.3 | 539.3 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 522.0 | 522.0 | 0.0 | 3.00 | `_int_malloc` |
| 512.0 | 512.0 | -0.0 | 102.40 | `__aarch64_ldadd8_relax` |
| 503.3 | 501.8 | -1.5 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 484.0 | 484.0 | 0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.4 | 468.0 | -0.4 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 384.3 | 384.3 | 0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 373.2 | 377.5 | 4.2 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 398.3 | 34.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 348.8 | 348.8 | 0.0 | 69.76 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 338.0 | 338.0 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | 0.0 | 63.76 | `__aarch64_swp4_rel` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.0 | 0.0 | -286.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.8 | 250.9 | -33.9 | 7.34 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 274.0 | 274.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 268.0 | 318.0 | 50.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 255.3 | 255.3 | 0.0 | 5.01 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 233.4 | -20.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 240.0 | 242.0 | 2.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 237.0 | 237.0 | 0.0 | 2.00 | `malloc_consolidate` |
| 235.8 | 213.8 | -22.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 235.4 | 215.3 | -20.0 | 47.07 | `__aarch64_cas8_acq_rel` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 212.3 | 212.4 | 0.1 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 194.4 | 194.4 | 0.0 | 8.00 | `counts::main` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,155.3 | 750.3 | -405.0 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 745.3 | 417.3 | -328.0 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 286.0 | 0.0 | -286.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 14.0 | 113.0 | 99.0 | `<bytes::bytes_mut::BytesMut>::with_capacity` |
| 71.0 | 166.0 | 95.0 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 28.0 | 107.0 | 79.0 | `<smallvec::SmallVec<[u8; 6]>>::from_slice` |
| 155.0 | 102.0 | -53.0 | `bacnet_server::server::requests::issue::response_limits` |
| 268.0 | 318.0 | 50.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 37.0 | 81.0 | 44.0 | `bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}` |
| 93.0 | 134.0 | 41.0 | `<tokio::sync::notify::Notified as core::ops::drop::Drop>::drop` |
| 86.0 | 45.0 | -41.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 90.0 | 50.0 | -40.0 | `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop` |
| 17.0 | 53.0 | 36.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}>` |
| 364.2 | 398.3 | 34.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 284.8 | 250.9 | -33.9 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 1,210.0 | 1,177.0 | -33.0 | `free` |
| 124.0 | 99.0 | -25.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 136.0 | 159.0 | 23.0 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 235.8 | 213.8 | -22.0 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 93.0 | 72.0 | -21.0 | `<bacnet_client::tsm::Tsm>::complete_transaction_inner` |
| 235.4 | 215.3 | -20.0 | `__aarch64_cas8_acq_rel` |
| 0.0 | 20.0 | 20.0 | `<bacnet_client::sync_mutex::SyncMutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock` |
| 253.4 | 233.4 | -20.0 | `__aarch64_ldadd8_rel` |
| 0.0 | 17.0 | 17.0 | `core::ptr::drop_glue::<tokio::util::sharded_list::ShardGuard<tokio::runtime::task::Task<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>>>` |
| 0.0 | 16.0 | 16.0 | `<core::result::Result<(u8, bacnet_client::tsm::TransactionRegistration), bacnet_client::tsm::coordinated::CoordinatedRegistrationError>>::map_err::<bacnet_types` |
| 12.0 | 0.0 | -12.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}>` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::take_current_reassembly::{closure#0}>` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}>` |
| 109.0 | 100.0 | -9.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::current_reassembly_owner::{closure#0}>` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 999.0 | 988.0 | -11.0 | `library/core/src/ptr/mod.rs:573` |
| 933.0 | 883.0 | -50.0 | `library/core/src/ptr/mod.rs:847` |
| 743.7 | 727.7 | -16.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 523.5 | 507.5 | -16.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 425.2 | 432.1 | 7.0 | `library/core/src/sync/atomic.rs:4029` |
| 375.9 | 376.9 | 1.0 | `library/core/src/sync/atomic.rs:3994` |
| 312.0 | 312.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 306.2 | 306.2 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 288.8 | 285.8 | -3.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.1 | 283.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 270.0 | 251.5 | -18.5 | `library/core/src/cell.rs:558` |
| 268.1 | 260.1 | -8.0 | `library/core/src/option.rs:743` |
| 251.1 | 247.1 | -4.0 | `library/core/src/mem/mod.rs:976` |
| 247.0 | 247.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 224.0 | 229.0 | 5.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 190.3 | 174.2 | -16.0 | `library/core/src/sync/atomic.rs:4104` |
| 185.9 | 181.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 165.4 | 165.4 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 160.0 | 113.0 | -47.0 | `library/core/src/result.rs:0` |
| 160.0 | 158.0 | -2.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 159.0 | 146.9 | -12.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 156.0 | 156.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 155.3 | 152.3 | -3.0 | `library/core/src/sync/atomic.rs:4013` |
| 151.0 | 151.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 149.5 | 145.5 | -4.0 | `library/core/src/sync/atomic.rs:3995` |
| 148.1 | 96.1 | -52.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 133.9 | 126.9 | -7.0 | `library/core/src/cell.rs:2437` |
| 130.9 | 126.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 130.0 | 130.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 129.0 | 129.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 128.0 | 128.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 125.0 | 125.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.0 | 77.0 | -44.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 120.5 | 120.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 109.0 | 109.0 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 102.1 | 102.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 99.8 | 99.8 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
