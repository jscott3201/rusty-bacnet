# wp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,634.0 | 1,634.0 | 0.0 | 24.00 | `_int_free` |
| 1,621.0 | 1,621.0 | 0.0 | 50.00 | `__GI_memcpy` |
| 1,133.0 | 1,133.0 | 0.0 | 21.00 | `malloc` |
| 1,101.0 | 1,101.0 | 0.0 | 24.00 | `free` |
| 954.7 | 832.5 | -122.1 | 15.67 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.2 | -0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 750.2 | 750.3 | 0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 697.6 | 697.5 | -0.1 | 8.20 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 612.0 | 612.0 | 0.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | 0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 532.0 | 736.0 | 204.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 522.0 | 522.0 | 0.0 | 3.00 | `_int_malloc` |
| 497.9 | 496.8 | -1.1 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 492.0 | 492.0 | 0.0 | 98.39 | `__aarch64_ldadd8_relax` |
| 483.0 | 483.0 | 0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 468.3 | 468.3 | -0.0 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.2 | 417.2 | 0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 398.5 | 398.5 | -0.0 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 384.2 | 320.2 | -64.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 377.8 | 377.8 | 0.1 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 348.8 | 348.8 | 0.0 | 69.76 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 337.9 | 337.9 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | 0.0 | 63.76 | `__aarch64_swp4_rel` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 274.0 | 274.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.2 | 255.2 | 0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 251.0 | 250.9 | -0.1 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 237.0 | 237.0 | 0.0 | 2.00 | `malloc_consolidate` |
| 234.0 | 0.0 | -234.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | 0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 229.0 | 229.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 215.3 | 215.3 | 0.0 | 43.06 | `__aarch64_cas8_acq_rel` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.3 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 194.4 | 194.4 | -0.0 | 8.00 | `counts::main` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 188.6 | 190.2 | 1.6 | 1.98 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 188.0 | 188.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 234.0 | 0.0 | -234.0 | `<bacnet_time::host::HostClock>::now` |
| 532.0 | 736.0 | 204.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 954.7 | 832.5 | -122.1 | `<std::sys::pal::unix::time::Timespec>::now` |
| 384.2 | 320.2 | -64.0 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 160.1 | 140.1 | -20.0 | `clock_gettime@@GLIBC_2.17` |
| 4.0 | 0.0 | -4.0 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 188.6 | 190.2 | 1.6 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 497.9 | 496.8 | -1.1 | `<tokio::runtime::time::Driver>::park_internal` |
| 137.1 | 137.5 | 0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 96.7 | 97.0 | 0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 865.4 | 865.2 | -0.2 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 76.8 | 76.6 | -0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 251.0 | 250.9 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 109.8 | 109.7 | -0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 51.3 | 51.4 | 0.1 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 377.8 | 377.8 | 0.1 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 44.0 | 44.1 | 0.1 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 34.4 | 34.4 | -0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 87.9 | 87.8 | -0.1 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 697.6 | 697.5 | -0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 233.4 | 233.4 | 0.0 | `__aarch64_ldadd8_rel` |
| 117.6 | 117.6 | -0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 121.7 | 121.6 | -0.0 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 398.5 | 398.5 | -0.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.3 | 468.3 | -0.0 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 164.0 | 164.0 | 0.0 | `tokio::runtime::task::waker::clone_waker` |
| 107.9 | 107.9 | 0.0 | `tokio::runtime::task::waker::drop_waker` |
| 101.0 | 101.0 | -0.0 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 337.9 | 337.9 | 0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 215.3 | 215.3 | 0.0 | `__aarch64_cas8_acq_rel` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 955.0 | 955.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 837.9 | 837.9 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 711.7 | 711.7 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 507.4 | 507.5 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 410.2 | 410.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 374.8 | 374.9 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 288.0 | 288.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 285.8 | 285.8 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.0 | 283.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 272.1 | 238.1 | -34.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 256.1 | 256.1 | 0.0 | `library/core/src/option.rs:743` |
| 249.5 | 249.5 | 0.0 | `library/core/src/cell.rs:558` |
| 247.0 | 247.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 244.0 | 244.0 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 217.0 | 217.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 189.0 | 189.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 177.9 | 177.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.2 | 174.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 164.3 | 164.3 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 152.3 | 152.3 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 150.0 | 150.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 146.9 | 146.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 145.5 | 145.5 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 144.0 | 144.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 139.0 | 139.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.9 | 126.9 | 0.0 | `library/core/src/cell.rs:2437` |
| 126.9 | 126.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 121.0 | 121.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.5 | 120.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 120.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 118.0 | 118.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 107.0 | 107.0 | 0.0 | `library/core/src/result.rs:0` |
| 105.0 | 105.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 103.0 | 103.0 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 102.0 | 102.0 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 102.0 | 102.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 99.8 | 99.8 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 96.0 | 96.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
