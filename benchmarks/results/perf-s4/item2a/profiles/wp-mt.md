# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,760.8 | 1,756.6 | -4.3 | 26.01 | `_int_free` |
| 1,651.0 | 1,651.0 | 0.0 | 52.00 | `__GI_memcpy` |
| 1,272.7 | 1,237.7 | -35.0 | 26.00 | `free` |
| 1,248.1 | 1,247.1 | -1.1 | 23.04 | `malloc` |
| 1,157.3 | 750.5 | -406.8 | 11.03 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 946.3 | 944.2 | -2.1 | 14.17 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.8 | 865.6 | -0.2 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 748.7 | 417.5 | -331.2 | 9.03 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 659.0 | 659.0 | -0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.5 | 634.9 | -0.6 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 568.3 | 567.8 | -0.5 | 14.04 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 539.7 | 539.4 | -0.3 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 512.2 | 512.0 | -0.2 | 102.43 | `__aarch64_ldadd8_relax` |
| 506.5 | 505.3 | -1.2 | 97.27 | `__aarch64_cas4_acq` |
| 500.8 | 491.8 | -9.1 | 3.59 | `_int_malloc` |
| 484.5 | 484.1 | -0.4 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.2 | 468.3 | 0.1 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 426.0 | 424.6 | -1.4 | 1.32 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 379.8 | 378.7 | -1.1 | 75.16 | `__aarch64_swp4_rel` |
| 379.5 | 378.7 | -0.8 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 374.2 | 379.4 | 5.2 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.3 | 398.4 | 34.1 | 7.76 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 357.9 | 357.3 | -0.6 | 11.18 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 338.5 | 338.2 | -0.3 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.3 | 319.1 | -0.2 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.7 | 0.0 | -286.7 | 4.04 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 281.1 | 280.7 | -0.4 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 271.4 | 250.4 | -21.0 | 54.28 | `__aarch64_cas8_acq_rel` |
| 268.6 | 317.9 | 49.4 | 6.04 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 233.4 | -20.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 243.2 | 250.8 | 7.6 | 6.30 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 240.0 | 242.0 | 2.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 235.8 | 213.8 | -22.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 234.0 | 234.0 | -0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 212.9 | 212.2 | -0.8 | 2.08 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 212.9 | 212.6 | -0.3 | 3.01 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 204.3 | 204.2 | -0.1 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 197.8 | 197.3 | -0.6 | 4.03 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.3 | 193.2 | -0.1 | 8.05 | `__pthread_enable_asynccancel` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,157.3 | 750.5 | -406.8 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 748.7 | 417.5 | -331.2 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 286.7 | 0.0 | -286.7 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 14.0 | 113.0 | 99.0 | `<bytes::bytes_mut::BytesMut>::with_capacity` |
| 71.0 | 166.0 | 95.0 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 28.0 | 107.0 | 79.0 | `<smallvec::SmallVec<[u8; 6]>>::from_slice` |
| 155.0 | 102.0 | -53.0 | `bacnet_server::server::requests::issue::response_limits` |
| 268.6 | 317.9 | 49.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 37.0 | 81.0 | 44.0 | `bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}` |
| 86.0 | 45.0 | -41.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 90.1 | 50.1 | -40.0 | `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop` |
| 17.0 | 53.0 | 36.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}>` |
| 1,272.7 | 1,237.7 | -35.0 | `free` |
| 364.3 | 398.4 | 34.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 124.0 | 99.0 | -25.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 135.0 | 159.0 | 24.0 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 29.0 | 52.5 | 23.5 | `_int_free_maybe_consolidate.part.0` |
| 235.8 | 213.8 | -22.0 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 271.4 | 250.4 | -21.0 | `__aarch64_cas8_acq_rel` |
| 92.4 | 72.0 | -20.4 | `<bacnet_client::tsm::Tsm>::complete_transaction_inner` |
| 0.0 | 20.0 | 20.0 | `<bacnet_client::sync_mutex::SyncMutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock` |
| 253.4 | 233.4 | -20.0 | `__aarch64_ldadd8_rel` |
| 0.0 | 17.0 | 17.0 | `core::ptr::drop_glue::<tokio::util::sharded_list::ShardGuard<tokio::runtime::task::Task<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>>>` |
| 0.0 | 16.0 | 16.0 | `<core::result::Result<(u8, bacnet_client::tsm::TransactionRegistration), bacnet_client::tsm::coordinated::CoordinatedRegistrationError>>::map_err::<bacnet_types` |
| 12.0 | 0.0 | -12.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}>` |
| 500.8 | 491.8 | -9.1 | `_int_malloc` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::current_reassembly_owner::{closure#0}>` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::take_current_reassembly::{closure#0}>` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}>` |
| 109.0 | 100.0 | -9.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 999.1 | 988.1 | -11.0 | `library/core/src/ptr/mod.rs:573` |
| 915.9 | 866.4 | -49.5 | `library/core/src/ptr/mod.rs:847` |
| 772.1 | 755.6 | -16.5 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 590.1 | 572.3 | -17.8 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 428.3 | 435.1 | 6.9 | `library/core/src/sync/atomic.rs:4029` |
| 400.7 | 400.9 | 0.1 | `library/core/src/sync/atomic.rs:3994` |
| 315.3 | 311.4 | -3.9 | `library/core/src/sync/atomic.rs:4083` |
| 312.1 | 312.0 | -0.1 | `./malloc/./malloc/malloc.c:4518` |
| 311.3 | 310.7 | -0.7 | `library/std/src/sys/pal/unix/time.rs:107` |
| 308.7 | 307.8 | -0.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 272.4 | 271.6 | -0.8 | `library/std/src/panicking.rs:465` |
| 265.8 | 257.5 | -8.3 | `library/core/src/option.rs:743` |
| 255.6 | 236.8 | -18.8 | `library/core/src/cell.rs:558` |
| 232.0 | 227.6 | -4.4 | `library/core/src/mem/mod.rs:976` |
| 225.4 | 224.1 | -1.3 | `???:0` |
| 224.0 | 229.0 | 5.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 193.0 | 188.9 | -4.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 190.8 | 174.3 | -16.5 | `library/core/src/sync/atomic.rs:4104` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 169.2 | 165.6 | -3.6 | `library/core/src/sync/atomic.rs:4013` |
| 167.0 | 155.0 | -12.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 166.7 | 162.4 | -4.3 | `library/core/src/sync/atomic.rs:3995` |
| 164.6 | 164.4 | -0.2 | `library/core/src/ptr/non_null.rs:454` |
| 160.3 | 113.0 | -47.3 | `library/core/src/result.rs:0` |
| 160.0 | 158.0 | -2.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.1 | 156.0 | -0.1 | `./malloc/./malloc/malloc.c:4495` |
| 152.0 | 144.7 | -7.3 | `library/core/src/cell.rs:2437` |
| 151.8 | 151.5 | -0.3 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 148.4 | 96.1 | -52.3 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 147.5 | 143.1 | -4.4 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 139.2 | 138.8 | -0.4 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.1 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.1 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.1 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.1 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.4 | 129.0 | -0.3 | `./malloc/./malloc/malloc.c:1336` |
| 128.1 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 125.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.3 | 77.1 | -44.2 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 115.0 | 115.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.5 | 105.9 | 0.4 | `./malloc/./malloc/arena.c:147` |
| 105.3 | 105.1 | -0.2 | `library/core/src/ptr/mod.rs:1755` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 99.8 | 99.8 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
