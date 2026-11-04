# wp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,764.0 | 1,764.0 | 0.0 | 26.00 | `_int_free` |
| 1,651.0 | 1,651.0 | 0.0 | 52.00 | `__GI_memcpy` |
| 1,241.0 | 1,241.0 | 0.0 | 23.00 | `malloc` |
| 1,169.0 | 1,169.0 | -0.0 | 26.00 | `free` |
| 954.3 | 954.3 | 0.0 | 15.68 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.3 | -0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 750.2 | 750.2 | 0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 697.5 | 697.5 | -0.1 | 8.20 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 664.0 | 664.0 | 0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | 0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 522.0 | 522.0 | 0.0 | 3.00 | `_int_malloc` |
| 511.9 | 511.9 | 0.0 | 102.38 | `__aarch64_ldadd8_relax` |
| 495.2 | 495.5 | 0.3 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 483.0 | 483.0 | 0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 467.7 | 467.8 | 0.1 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.2 | 417.2 | 0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.4 | 398.5 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 384.2 | 384.2 | 0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 377.5 | 377.3 | -0.2 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 348.8 | 348.8 | -0.0 | 69.75 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 337.9 | 337.9 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | -0.0 | 63.75 | `__aarch64_swp4_rel` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 274.0 | 274.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.2 | 255.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 251.1 | 251.1 | 0.0 | 5.34 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 237.0 | 237.0 | 0.0 | 2.00 | `malloc_consolidate` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | 0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 228.0 | 228.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 215.3 | 215.3 | 0.0 | 43.05 | `__aarch64_cas8_acq_rel` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.3 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 194.4 | 194.4 | -0.0 | 8.00 | `counts::main` |
| 192.6 | 192.2 | -0.4 | 2.02 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 192.6 | 192.2 | -0.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 87.8 | 87.5 | -0.3 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 117.6 | 117.2 | -0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 495.2 | 495.5 | 0.3 | `<tokio::runtime::time::Driver>::park_internal` |
| 377.5 | 377.3 | -0.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 82.6 | 82.4 | -0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 100.8 | 100.7 | -0.1 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 92.9 | 93.0 | 0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 467.7 | 467.8 | 0.1 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 107.7 | 107.7 | -0.1 | `tokio::runtime::task::waker::drop_waker` |
| 163.8 | 163.7 | -0.1 | `tokio::runtime::task::waker::clone_waker` |
| 398.4 | 398.5 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 697.5 | 697.5 | -0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 954.3 | 954.3 | 0.0 | `<std::sys::pal::unix::time::Timespec>::now` |
| 137.0 | 137.1 | 0.0 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 865.4 | 865.3 | -0.0 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 76.4 | 76.4 | -0.0 | `<tokio::time::interval::Interval>::poll_tick` |
| 109.5 | 109.6 | 0.0 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 83.5 | 83.5 | -0.0 | `__aarch64_ldadd8_acq_rel` |
| 121.6 | 121.7 | 0.0 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 96.6 | 96.6 | -0.0 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 34.3 | 34.3 | 0.0 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 251.1 | 251.1 | 0.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 337.9 | 337.9 | 0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 215.3 | 215.3 | 0.0 | `__aarch64_cas8_acq_rel` |
| 255.2 | 255.2 | -0.0 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 51.4 | 51.4 | 0.0 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 44.0 | 44.0 | 0.0 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 107.8 | 107.8 | -0.0 | `tokio::runtime::task::raw::poll::<core::pin::Pin<alloc::boxed::Box<bacnet_server::server::audit_recipient::spawn_owned<<bacnet_server::server::BACnetServer<bacn` |
| 219.3 | 219.3 | 0.0 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 989.0 | 989.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 854.9 | 854.9 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 727.5 | 727.5 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 507.3 | 507.3 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 432.1 | 432.1 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 376.8 | 376.8 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 312.0 | 312.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 285.8 | 285.8 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.0 | 283.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 272.1 | 272.1 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 260.1 | 260.1 | -0.0 | `library/core/src/option.rs:743` |
| 249.4 | 249.4 | 0.0 | `library/core/src/cell.rs:558` |
| 247.0 | 247.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 244.0 | 244.0 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 229.0 | 229.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 181.9 | 181.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.2 | 174.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 164.3 | 164.3 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 158.0 | 158.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.0 | 156.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 152.3 | 152.3 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 151.0 | 151.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 146.9 | 146.9 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 145.4 | 145.4 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 129.0 | 129.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 128.0 | 128.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 126.8 | 126.8 | -0.0 | `library/core/src/cell.rs:2437` |
| 126.8 | 126.8 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 125.0 | 125.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 120.5 | 120.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 113.0 | 113.0 | 0.0 | `library/core/src/result.rs:0` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 109.0 | 109.0 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 103.0 | 103.0 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 99.7 | 99.7 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
