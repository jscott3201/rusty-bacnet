# wp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,685.0 | 1,685.0 | 0.0 | 50.00 | `__GI_memcpy` |
| 1,634.0 | 1,634.0 | 0.0 | 24.00 | `_int_free` |
| 1,133.0 | 1,133.0 | 0.0 | 21.00 | `malloc` |
| 1,106.0 | 1,106.0 | 0.0 | 24.00 | `free` |
| 894.8 | 804.8 | -90.0 | 14.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 878.7 | 878.9 | 0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 744.2 | 742.2 | -2.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 697.6 | 697.6 | -0.0 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 612.0 | 612.0 | 0.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | 0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.8 | 545.8 | 0.0 | 3.00 | `_int_malloc` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 498.8 | 498.6 | -0.2 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 480.0 | 485.0 | 5.0 | 9.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 479.5 | 479.4 | -0.0 | 95.89 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.2 | 417.2 | 0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 352.2 | 320.2 | -32.0 | 11.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 348.8 | 348.8 | -0.0 | 69.75 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 337.9 | 337.9 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | -0.0 | 63.75 | `__aarch64_swp4_rel` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 296.7 | 296.5 | -0.2 | 6.48 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 290.2 | 289.8 | -0.4 | 5.94 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 274.0 | 274.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.2 | 255.2 | 0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 251.0 | 251.0 | 0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 238.4 | 238.4 | -0.0 | 47.68 | `__aarch64_ldadd8_rel` |
| 237.6 | 237.6 | 0.0 | 2.00 | `malloc_consolidate` |
| 229.0 | 229.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 224.0 | 224.0 | 0.0 | 5.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.3 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 197.8 | 197.8 | -0.0 | 39.56 | `__aarch64_cas8_acq_rel` |
| 194.4 | 194.4 | 0.0 | 8.00 | `counts::main` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 188.0 | 188.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 187.2 | 187.5 | 0.3 | 1.97 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 180.0 | 180.0 | 0.0 | 2.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 894.8 | 804.8 | -90.0 | `<std::sys::pal::unix::time::Timespec>::now` |
| 35.0 | 0.0 | -35.0 | `<bacnet_time::host::HostClock>::now` |
| 352.2 | 320.2 | -32.0 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 150.1 | 130.1 | -20.0 | `clock_gettime@@GLIBC_2.17` |
| 43.0 | 63.0 | 20.0 | `<bacnet_server::cov::CovSubscriptionTable>::timed_capture` |
| 480.0 | 485.0 | 5.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 744.2 | 742.2 | -2.0 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 2.0 | 0.0 | -2.0 | `<std::time::Instant>::now` |
| 101.2 | 100.8 | -0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 290.2 | 289.8 | -0.4 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 187.2 | 187.5 | 0.3 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 296.7 | 296.5 | -0.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 498.8 | 498.6 | -0.2 | `<tokio::runtime::time::Driver>::park_internal` |
| 82.8 | 82.6 | -0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 878.7 | 878.9 | 0.2 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 136.9 | 136.8 | -0.1 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 96.6 | 96.5 | -0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 115.0 | 114.9 | -0.1 | `tokio::runtime::task::waker::clone_waker` |
| 121.7 | 121.7 | -0.1 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 58.9 | 58.8 | -0.1 | `tokio::runtime::task::waker::drop_waker` |
| 51.4 | 51.3 | -0.1 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 44.0 | 44.0 | -0.0 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 479.5 | 479.4 | -0.0 | `__aarch64_ldadd8_relax` |
| 66.1 | 66.0 | -0.0 | `__aarch64_ldadd8_acq_rel` |
| 251.0 | 251.0 | 0.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 109.9 | 109.9 | -0.0 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 42.4 | 42.4 | -0.0 | `__aarch64_cas8_acq` |
| 197.8 | 197.8 | -0.0 | `__aarch64_cas8_acq_rel` |
| 238.4 | 238.4 | -0.0 | `__aarch64_ldadd8_rel` |
| 318.8 | 318.8 | -0.0 | `__aarch64_swp4_rel` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 954.0 | 954.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 837.4 | 837.4 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 691.7 | 691.6 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 479.4 | 479.4 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 401.7 | 401.6 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 374.8 | 374.8 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 288.0 | 288.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 285.8 | 285.7 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.0 | 283.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 255.1 | 221.1 | -34.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 250.8 | 250.8 | -0.0 | `library/core/src/option.rs:743` |
| 247.0 | 247.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 221.5 | 221.5 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 220.5 | 220.4 | -0.0 | `library/core/src/cell.rs:558` |
| 220.0 | 220.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 189.0 | 189.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 172.9 | 172.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 162.3 | 162.3 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 156.7 | 156.7 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 152.3 | 152.3 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 150.0 | 150.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 148.2 | 140.2 | -8.0 | `library/core/src/sync/atomic.rs:3995` |
| 144.0 | 144.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 139.0 | 139.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 133.4 | 133.4 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 130.0 | 130.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 121.0 | 121.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.5 | 120.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 120.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 119.8 | 119.8 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 118.0 | 118.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 115.1 | 115.1 | -0.0 | `library/core/src/cell.rs:2437` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 107.0 | 107.0 | 0.0 | `library/core/src/result.rs:0` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 105.0 | 105.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.0 | 105.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 104.3 | 104.3 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 102.0 | 102.0 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 102.0 | 102.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 96.0 | 96.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
