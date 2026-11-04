# wp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,634.0 | 1,569.0 | -65.0 | 24.00 | `_int_free` |
| 1,621.0 | 1,495.0 | -126.0 | 50.00 | `__GI_memcpy` |
| 1,155.4 | 1,155.4 | 0.0 | 11.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 1,142.0 | 1,109.0 | -33.0 | 24.00 | `free` |
| 1,133.0 | 1,085.0 | -48.0 | 21.00 | `malloc` |
| 1,026.0 | 1,025.7 | -0.3 | 17.63 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.5 | 865.3 | -0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.3 | 745.3 | 0.0 | 9.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 634.7 | 634.7 | 0.0 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 612.0 | 586.0 | -26.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.6 | 567.6 | 0.0 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.3 | 539.3 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 532.0 | 532.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 522.0 | 547.9 | 25.9 | 3.00 | `_int_malloc` |
| 505.3 | 503.3 | -2.0 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 492.0 | 482.0 | -10.0 | 98.40 | `__aarch64_ldadd8_relax` |
| 484.0 | 481.0 | -3.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.4 | 468.2 | -0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 384.3 | 384.3 | 0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 373.1 | 373.0 | -0.1 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.4 | 364.3 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 348.8 | 348.8 | 0.0 | 69.76 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.0 | 338.0 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | 0.0 | 63.76 | `__aarch64_swp4_rel` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 284.0 | -9.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.0 | 286.0 | 0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.7 | 284.7 | -0.0 | 7.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 274.0 | 0.0 | -274.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 268.0 | 268.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 255.3 | 255.3 | 0.0 | 5.01 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 253.4 | -0.0 | 50.68 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 240.0 | 240.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 237.0 | 237.4 | 0.4 | 2.00 | `malloc_consolidate` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 235.4 | 235.3 | -0.0 | 47.08 | `__aarch64_cas8_acq_rel` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 212.4 | 212.4 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 194.4 | 194.4 | 0.0 | 8.01 | `counts::main` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 202.0 | 13.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 187.0 | 187.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 274.0 | 0.0 | -274.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 253.0 | 253.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_ge` |
| 0.0 | 135.0 | 135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 135.0 | 0.0 | -135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 1,621.0 | 1,495.0 | -126.0 | `__GI_memcpy` |
| 95.0 | 0.0 | -95.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 91.0 | 91.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_ge` |
| 0.0 | 82.9 | 82.9 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 82.9 | 0.0 | -82.9 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 1,634.0 | 1,569.0 | -65.0 | `_int_free` |
| 0.0 | 57.0 | 57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_t` |
| 57.0 | 0.0 | -57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_` |
| 1,133.0 | 1,085.0 | -48.0 | `malloc` |
| 44.0 | 0.0 | -44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::ser` |
| 0.0 | 44.0 | 44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::b` |
| 41.0 | 0.0 | -41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server` |
| 0.0 | 41.0 | 41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpo` |
| 1,142.0 | 1,109.0 | -33.0 | `free` |
| 612.0 | 586.0 | -26.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 522.0 | 547.9 | 25.9 | `_int_malloc` |
| 0.0 | 22.0 | 22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bac` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<ba` |
| 0.0 | 14.0 | 14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_trans` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_general::{closure#0}>` |
| 14.0 | 0.0 | -14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_serv` |
| 14.0 | 0.0 | -14.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>` |
| 189.0 | 202.0 | 13.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 13.0 | 0.0 | -13.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BACnetServer` |
| 0.0 | 11.0 | 11.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>` |
| 492.0 | 482.0 | -10.0 | `__aarch64_ldadd8_relax` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 965.0 | 965.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 926.0 | 921.0 | -5.0 | `library/core/src/ptr/mod.rs:847` |
| 727.7 | 719.7 | -8.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 523.5 | 523.5 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 403.2 | 392.2 | -11.0 | `library/core/src/sync/atomic.rs:4029` |
| 373.9 | 373.9 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 306.2 | 306.2 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 288.8 | 288.8 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 288.0 | 276.0 | -12.0 | `./malloc/./malloc/malloc.c:4518` |
| 283.1 | 283.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 270.0 | 270.0 | -0.0 | `library/core/src/cell.rs:558` |
| 264.1 | 264.1 | 0.0 | `library/core/src/option.rs:743` |
| 251.1 | 251.1 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 247.0 | 247.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 212.0 | 212.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 190.3 | 190.2 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 189.0 | 180.0 | -9.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 181.9 | 179.9 | -2.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 165.4 | 165.4 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 159.0 | 159.0 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 155.3 | 155.3 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 154.0 | 154.0 | 0.0 | `library/core/src/result.rs:0` |
| 152.0 | 148.0 | -4.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 149.5 | 149.5 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 148.1 | 148.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 144.0 | 138.0 | -6.0 | `./malloc/./malloc/malloc.c:4495` |
| 139.0 | 133.0 | -6.0 | `./malloc/./malloc/malloc.c:4527` |
| 133.9 | 133.9 | -0.0 | `library/core/src/cell.rs:2437` |
| 130.9 | 130.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 128.0 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 120.0 | -6.0 | `./malloc/./malloc/malloc.c:3294` |
| 126.0 | 120.0 | -6.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 120.0 | -6.0 | `./malloc/./malloc/malloc.c:3354` |
| 121.0 | 121.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 121.0 | 117.0 | -4.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.5 | 120.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 115.0 | -5.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 118.0 | 113.0 | -5.0 | `./malloc/./malloc/malloc.c:4658` |
| 115.0 | 110.0 | -5.0 | `./malloc/./malloc/malloc.c:4547` |
| 105.0 | 100.0 | -5.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 103.0 | 103.0 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 102.1 | 102.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 102.0 | 98.0 | -4.0 | `./malloc/./malloc/arena.c:153` |
| 99.8 | 99.8 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 96.0 | 92.0 | -4.0 | `./malloc/./malloc/malloc.c:4510` |
