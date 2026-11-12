# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,685.0 | 1,685.0 | 0.0 | 50.00 | `__GI_memcpy` |
| 1,619.0 | 1,618.7 | -0.3 | 24.00 | `_int_free` |
| 1,158.5 | 1,159.2 | 0.7 | 24.00 | `free` |
| 1,139.1 | 1,139.1 | 0.0 | 21.00 | `malloc` |
| 865.5 | 878.9 | 13.5 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 813.2 | 813.3 | 0.1 | 11.13 | `<std::sys::pal::unix::time::Timespec>::now` |
| 744.3 | 744.3 | -0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 698.0 | 697.7 | -0.2 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 607.0 | 607.0 | 0.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | -0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 505.1 | 505.1 | 0.0 | 96.99 | `__aarch64_cas4_acq` |
| 496.9 | 479.5 | -17.5 | 99.39 | `__aarch64_ldadd8_relax` |
| 480.1 | 480.2 | 0.1 | 9.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 467.8 | 296.5 | -171.3 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 461.4 | 460.2 | -1.2 | 3.12 | `_int_malloc` |
| 424.5 | 426.2 | 1.8 | 1.32 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.3 | 417.3 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 398.4 | 224.0 | -174.4 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 378.6 | 291.0 | -87.6 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.5 | 378.5 | 0.0 | 74.90 | `__aarch64_swp4_rel` |
| 378.2 | 379.2 | 1.0 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 338.0 | 338.0 | 0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 324.9 | 325.0 | 0.1 | 10.15 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 319.0 | 319.0 | 0.1 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 318.0 | 318.0 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | -0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.4 | 280.4 | -0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 251.0 | 251.1 | 0.1 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 250.2 | 232.7 | -17.5 | 50.04 | `__aarch64_cas8_acq_rel` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 238.4 | 238.4 | 0.0 | 47.68 | `__aarch64_ldadd8_rel` |
| 229.0 | 229.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.3 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 211.3 | 210.9 | -0.4 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.2 | 204.1 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 197.3 | 197.3 | 0.0 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.1 | 193.1 | 0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 188.0 | 188.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 398.4 | 224.0 | -174.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 467.8 | 296.5 | -171.3 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.4 | 0.0 | -117.4 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.7 | 0.0 | -87.7 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 378.6 | 291.0 | -87.6 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 76.4 | 0.0 | -76.3 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.0 | 72.0 | 72.0 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.0 | 63.0 | 63.0 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closure#0}::{cl` |
| 164.0 | 115.0 | -49.0 | `tokio::runtime::task::waker::clone_waker` |
| 107.9 | 58.9 | -49.0 | `tokio::runtime::task::waker::drop_waker` |
| 92.9 | 62.0 | -30.9 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 250.2 | 232.7 | -17.5 | `__aarch64_cas8_acq_rel` |
| 113.3 | 95.8 | -17.5 | `__aarch64_ldadd8_acq_rel` |
| 65.9 | 48.4 | -17.5 | `__aarch64_cas8_acq` |
| 496.9 | 479.5 | -17.5 | `__aarch64_ldadd8_relax` |
| 865.5 | 878.9 | 13.5 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 52.9 | 46.8 | -6.1 | `_int_free_maybe_consolidate.part.0` |
| 126.9 | 124.4 | -2.5 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 3.8 | 6.1 | 2.2 | `unlink_chunk.isra.0` |
| 424.5 | 426.2 | 1.8 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 21.0 | 22.8 | 1.7 | `_int_free_create_chunk` |
| 143.3 | 142.0 | -1.3 | `malloc_consolidate` |
| 461.4 | 460.2 | -1.2 | `_int_malloc` |
| 68.0 | 67.0 | -1.0 | `counts::main::{closure#0}` |
| 378.2 | 379.2 | 1.0 | `<tokio::runtime::time::Driver>::park_internal` |
| 1,158.5 | 1,159.2 | 0.7 | `free` |
| 211.3 | 210.9 | -0.4 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 6.6 | 6.2 | -0.4 | `alloc_perturb` |
| 136.7 | 137.0 | 0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 1,619.0 | 1,618.7 | -0.3 | `_int_free` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 954.0 | 954.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 819.2 | 820.7 | 1.5 | `library/core/src/ptr/mod.rs:847` |
| 747.5 | 719.6 | -28.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 572.1 | 544.1 | -28.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 415.2 | 404.7 | -10.5 | `library/core/src/sync/atomic.rs:4029` |
| 398.6 | 398.6 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 311.4 | 311.4 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 307.6 | 307.6 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 288.0 | 288.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 271.6 | 271.6 | 0.0 | `library/std/src/panicking.rs:465` |
| 259.3 | 259.4 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 253.4 | 248.1 | -5.3 | `library/core/src/option.rs:743` |
| 234.7 | 205.7 | -29.0 | `library/core/src/cell.rs:558` |
| 224.5 | 202.0 | -22.5 | `library/core/src/mem/mod.rs:976` |
| 222.6 | 222.6 | -0.0 | `???:0` |
| 220.0 | 220.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 189.0 | 189.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 186.9 | 179.9 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.2 | 156.7 | -17.5 | `library/core/src/sync/atomic.rs:4104` |
| 170.3 | 165.1 | -5.2 | `library/core/src/sync/atomic.rs:3995` |
| 165.6 | 165.6 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 157.3 | 159.5 | 2.3 | `library/core/src/ptr/non_null.rs:454` |
| 155.0 | 141.4 | -13.5 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 150.3 | 148.3 | -2.0 | `library/core/src/num/uint_macros.rs:2652` |
| 150.0 | 150.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 144.6 | 132.9 | -11.7 | `library/core/src/cell.rs:2437` |
| 144.0 | 144.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 143.0 | 136.0 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 139.0 | 139.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 138.8 | 138.8 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 130.0 | 130.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 121.0 | 121.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.0 | 120.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 120.0 | 120.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 120.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 120.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 120.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 107.0 | 107.0 | -0.0 | `library/core/src/result.rs:0` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 105.1 | 105.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.0 | 105.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 99.7 | 64.7 | -35.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.1 | 98.0 | -0.1 | `./malloc/./malloc/arena.c:147` |
| 96.1 | 96.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 96.0 | 96.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 96.0 | 96.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
