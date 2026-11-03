# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,750.3 | 1,750.1 | -0.1 | 26.00 | `_int_free` |
| 1,603.0 | 1,651.0 | 48.0 | 49.00 | `__GI_memcpy` |
| 1,272.7 | 1,271.3 | -1.4 | 26.00 | `free` |
| 1,247.1 | 1,247.1 | -0.0 | 23.00 | `malloc` |
| 1,155.8 | 1,155.4 | -0.4 | 11.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 945.4 | 943.6 | -1.8 | 14.17 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.7 | 865.6 | -0.1 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 745.8 | 745.4 | -0.4 | 9.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 659.0 | 659.0 | -0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.3 | 634.8 | -0.5 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 568.1 | 567.4 | -0.6 | 14.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 539.6 | 539.2 | -0.4 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 521.0 | 0.0 | -521.0 | 4.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 512.0 | 511.9 | -0.0 | 102.39 | `__aarch64_ldadd8_relax` |
| 505.6 | 505.1 | -0.4 | 97.09 | `__aarch64_cas4_acq` |
| 484.5 | 484.2 | -0.3 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.1 | 467.9 | -0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 461.1 | 478.3 | 17.3 | 3.52 | `_int_malloc` |
| 425.7 | 427.0 | 1.3 | 1.33 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 379.5 | 379.7 | 0.2 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.9 | 378.5 | -0.4 | 74.98 | `__aarch64_swp4_rel` |
| 374.5 | 373.8 | -0.7 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 364.2 | -0.0 | 7.74 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 357.8 | 357.0 | -0.8 | 11.18 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 338.4 | 338.1 | -0.3 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.3 | 319.1 | -0.2 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | -0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.0 | 286.0 | -0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.9 | 280.4 | -0.5 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 270.6 | 270.3 | -0.3 | 54.12 | `__aarch64_cas8_acq_rel` |
| 268.0 | 268.0 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 253.5 | 253.4 | -0.1 | 50.69 | `__aarch64_ldadd8_rel` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 243.5 | 243.6 | 0.1 | 6.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 240.0 | 240.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 240.0 | 0.0 | -240.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 232.0 | 232.0 | -0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 220.0 | 220.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 212.8 | 212.4 | -0.4 | 3.01 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 212.7 | 211.0 | -1.7 | 2.08 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.3 | 204.2 | -0.1 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 521.0 | 0.0 | -521.0 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 240.0 | 0.0 | -240.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::effect::<f32>` |
| 0.0 | 175.0 | 175.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::replace` |
| 58.0 | 190.0 | 132.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 108.0 | 0.0 | -108.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::current::<f32>` |
| 1,603.0 | 1,651.0 | 48.0 | `__GI_memcpy` |
| 0.0 | 37.0 | 37.0 | `<bacnet_types::constructed::BACnetAddress as core::clone::Clone>::clone` |
| 26.0 | 0.0 | -26.0 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::record` |
| 461.1 | 478.3 | 17.3 | `_int_malloc` |
| 28.9 | 46.0 | 17.2 | `_int_free_maybe_consolidate.part.0` |
| 137.9 | 145.7 | 7.8 | `malloc_consolidate` |
| 27.8 | 23.0 | -4.9 | `_int_free_create_chunk` |
| 6.9 | 10.4 | 3.5 | `unlink_chunk.isra.0` |
| 3.8 | 6.1 | 2.3 | `alloc_perturb` |
| 127.0 | 124.8 | -2.2 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 945.4 | 943.6 | -1.8 | `<std::sys::pal::unix::time::Timespec>::now` |
| 212.7 | 211.0 | -1.7 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 1,272.7 | 1,271.3 | -1.4 | `free` |
| 425.7 | 427.0 | 1.3 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 48.9 | 49.9 | 1.0 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 22.0 | 21.0 | -1.0 | `<bacnet_server::life_safety_cov::LifeSafetyCovSnapshots>::capture_write_property` |
| 357.8 | 357.0 | -0.8 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 1.6 | 0.8 | -0.7 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 1.3 | 0.6 | -0.7 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 374.5 | 373.8 | -0.7 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 105.6 | 104.9 | -0.7 | `<tokio::runtime::scheduler::multi_thread::worker::Core>::maintenance` |
| 568.1 | 567.4 | -0.6 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 198.1 | 197.5 | -0.6 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 91.1 | 90.5 | -0.6 | `<tokio::runtime::time::handle::Handle>::reregister` |
| 0.9 | 0.3 | -0.6 | `<core::pin::Pin<alloc::boxed::Box<bacnet_server::server::audit_recipient::spawn_owned<bacnet_server::server::lifecycle::intrinsic::run<bacnet_transport::bip::Bi` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 918.7 | 915.3 | -3.3 | `library/core/src/ptr/mod.rs:847` |
| 909.1 | 999.0 | 90.0 | `library/core/src/ptr/mod.rs:573` |
| 771.7 | 771.5 | -0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 588.8 | 588.2 | -0.5 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 428.2 | 428.1 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 400.0 | 399.7 | -0.4 | `library/core/src/sync/atomic.rs:3994` |
| 314.7 | 314.4 | -0.3 | `library/core/src/sync/atomic.rs:4083` |
| 312.0 | 312.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 311.2 | 310.4 | -0.7 | `library/std/src/sys/pal/unix/time.rs:107` |
| 308.0 | 307.6 | -0.4 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 271.9 | 271.6 | -0.3 | `library/std/src/panicking.rs:465` |
| 255.4 | 255.2 | -0.2 | `library/core/src/cell.rs:558` |
| 249.6 | 265.4 | 15.9 | `library/core/src/option.rs:743` |
| 231.8 | 231.5 | -0.2 | `library/core/src/mem/mod.rs:976` |
| 224.0 | 224.0 | 0.0 | `library/core/src/result.rs:2177` |
| 222.7 | 222.6 | -0.0 | `???:0` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 192.9 | 192.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 190.4 | 190.2 | -0.1 | `library/core/src/sync/atomic.rs:4104` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 168.8 | 168.6 | -0.2 | `library/core/src/sync/atomic.rs:4013` |
| 167.1 | 167.0 | -0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 166.5 | 166.4 | -0.1 | `library/core/src/sync/atomic.rs:3995` |
| 164.5 | 164.3 | -0.2 | `library/core/src/ptr/non_null.rs:454` |
| 161.0 | 51.0 | -110.0 | `library/core/src/slice/iter/macros.rs:180` |
| 160.0 | 160.0 | -0.0 | `library/core/src/result.rs:0` |
| 160.0 | 160.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.0 | 156.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 151.8 | 151.7 | -0.1 | `library/core/src/cell.rs:2437` |
| 151.7 | 151.3 | -0.4 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 148.1 | 148.1 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 147.2 | 147.1 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.9 | 138.8 | -0.1 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 129.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 125.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.1 | 121.0 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 120.0 | 57.0 | -63.0 | `library/core/src/slice/iter/macros.rs:379` |
| 118.0 | 105.0 | -13.0 | `library/core/src/slice/cmp.rs:157` |
| 115.0 | 115.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 108.3 | 78.0 | -30.3 | `library/core/src/ptr/mod.rs:1963` |
| 105.5 | 105.8 | 0.3 | `./malloc/./malloc/arena.c:147` |
| 105.2 | 105.1 | -0.1 | `library/core/src/ptr/mod.rs:1755` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 104.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 99.8 | 99.7 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
