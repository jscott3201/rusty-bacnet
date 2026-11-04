# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,621.0 | 1,495.0 | -126.0 | 50.00 | `__GI_memcpy` |
| 1,618.2 | 1,555.0 | -63.2 | 24.00 | `_int_free` |
| 1,194.7 | 1,161.0 | -33.7 | 24.00 | `free` |
| 1,155.8 | 1,155.7 | -0.2 | 11.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 1,139.1 | 1,091.1 | -48.0 | 21.00 | `malloc` |
| 946.1 | 946.5 | 0.4 | 14.17 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.8 | 865.7 | -0.1 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.9 | 745.7 | -0.2 | 9.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.5 | 635.2 | -0.4 | 9.23 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 607.0 | 581.0 | -26.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 568.1 | 568.0 | -0.2 | 14.04 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.7 | 539.6 | -0.1 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 532.0 | 532.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 505.6 | 505.5 | -0.1 | 97.11 | `__aarch64_cas4_acq` |
| 492.2 | 482.1 | -10.0 | 98.43 | `__aarch64_ldadd8_relax` |
| 484.0 | 481.8 | -2.1 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 469.0 | 468.8 | -0.2 | 10.00 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 463.5 | 459.8 | -3.7 | 2.98 | `_int_malloc` |
| 429.0 | 426.0 | -3.0 | 1.38 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 382.4 | 380.1 | -2.3 | 4.11 | `<tokio::runtime::time::Driver>::park_internal` |
| 379.0 | 378.9 | -0.1 | 75.00 | `__aarch64_swp4_rel` |
| 374.9 | 374.6 | -0.2 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.6 | 364.5 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 358.0 | 357.8 | -0.1 | 11.19 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.5 | 338.4 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.4 | 319.4 | 0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 284.0 | -9.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.1 | 286.0 | -0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 281.0 | 280.8 | -0.1 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 0.0 | -280.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 270.7 | 270.6 | -0.1 | 54.14 | `__aarch64_cas8_acq_rel` |
| 267.9 | 267.9 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.4 | 253.4 | -0.0 | 50.69 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 243.4 | 243.5 | 0.0 | 6.32 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 240.0 | 240.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 234.0 | 234.0 | -0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 213.5 | 212.0 | -1.5 | 2.08 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 212.7 | 212.7 | -0.0 | 3.01 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 204.4 | 204.3 | -0.1 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 198.0 | 197.5 | -0.5 | 4.03 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.3 | 193.3 | 0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 280.0 | 0.0 | -280.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 259.0 | 259.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_ge` |
| 135.0 | 0.0 | -135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 0.0 | 135.0 | 135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 1,621.0 | 1,495.0 | -126.0 | `__GI_memcpy` |
| 95.0 | 0.0 | -95.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 91.0 | 91.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_ge` |
| 1,618.2 | 1,555.0 | -63.2 | `_int_free` |
| 60.0 | 0.0 | -60.0 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 0.0 | 60.0 | 60.0 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 57.0 | 0.0 | -57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_` |
| 0.0 | 57.0 | 57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_t` |
| 1,139.1 | 1,091.1 | -48.0 | `malloc` |
| 44.0 | 0.0 | -44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::ser` |
| 0.0 | 44.0 | 44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::b` |
| 41.0 | 0.0 | -41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server` |
| 0.0 | 41.0 | 41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpo` |
| 1,194.7 | 1,161.0 | -33.7 | `free` |
| 61.2 | 35.0 | -26.2 | `_int_free_maybe_consolidate.part.0` |
| 607.0 | 581.0 | -26.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<ba` |
| 0.0 | 22.0 | 22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bac` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_general::{closure#0}>` |
| 14.0 | 0.0 | -14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_serv` |
| 0.0 | 14.0 | 14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_trans` |
| 14.0 | 0.0 | -14.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>` |
| 13.0 | 0.0 | -13.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BACnetServer` |
| 189.0 | 202.0 | 13.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 0.0 | 11.0 | 11.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>` |
| 492.2 | 482.1 | -10.0 | `__aarch64_ldadd8_relax` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 965.1 | 965.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 908.7 | 903.6 | -5.1 | `library/core/src/ptr/mod.rs:847` |
| 756.1 | 748.0 | -8.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 589.0 | 588.8 | -0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 406.3 | 395.2 | -11.0 | `library/core/src/sync/atomic.rs:4029` |
| 398.2 | 398.0 | -0.2 | `library/core/src/sync/atomic.rs:3994` |
| 314.8 | 314.7 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 311.3 | 311.2 | -0.1 | `library/std/src/sys/pal/unix/time.rs:107` |
| 308.1 | 307.9 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 288.0 | 276.0 | -12.0 | `./malloc/./malloc/malloc.c:4518` |
| 272.0 | 271.9 | -0.1 | `library/std/src/panicking.rs:465` |
| 261.6 | 261.5 | -0.0 | `library/core/src/option.rs:743` |
| 255.5 | 255.4 | -0.1 | `library/core/src/cell.rs:558` |
| 231.8 | 231.8 | -0.1 | `library/core/src/mem/mod.rs:976` |
| 222.7 | 222.7 | -0.1 | `???:0` |
| 212.0 | 212.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 190.5 | 190.4 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 189.0 | 187.0 | -2.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 189.0 | 180.0 | -9.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 168.9 | 168.8 | -0.1 | `library/core/src/sync/atomic.rs:4013` |
| 167.1 | 167.1 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 166.5 | 166.5 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 158.6 | 158.5 | -0.1 | `library/core/src/ptr/non_null.rs:454` |
| 154.0 | 154.0 | -0.0 | `library/core/src/result.rs:0` |
| 152.0 | 148.0 | -4.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 151.9 | 151.9 | 0.0 | `library/core/src/cell.rs:2437` |
| 151.7 | 151.6 | -0.1 | `library/core/src/num/uint_macros.rs:2652` |
| 148.1 | 148.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 147.3 | 147.2 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 144.0 | 138.0 | -6.0 | `./malloc/./malloc/malloc.c:4495` |
| 139.0 | 133.0 | -6.0 | `./malloc/./malloc/malloc.c:4527` |
| 139.0 | 138.9 | -0.1 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 120.0 | -6.0 | `./malloc/./malloc/malloc.c:3294` |
| 126.0 | 120.0 | -6.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 120.0 | -6.0 | `./malloc/./malloc/malloc.c:3354` |
| 121.1 | 121.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 121.0 | 117.0 | -4.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:4658` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 115.0 | -5.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 115.0 | -5.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 115.0 | 110.0 | -5.0 | `./malloc/./malloc/malloc.c:4547` |
| 105.2 | 105.2 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.0 | 105.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 105.0 | 100.0 | -5.0 | `./malloc/./malloc/malloc.c:3311` |
| 100.0 | 99.9 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 86.0 | -13.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 98.4 | 95.6 | -2.8 | `./malloc/./malloc/arena.c:147` |
| 96.0 | 92.0 | -4.0 | `./malloc/./malloc/malloc.c:4505` |
