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
| 894.5 | 894.8 | 0.3 | 14.67 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.3 | 878.8 | 13.5 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 744.2 | 744.2 | -0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 697.6 | 697.7 | 0.1 | 8.20 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 612.0 | 612.0 | 0.0 | 22.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.4 | -0.0 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.2 | 545.8 | 0.6 | 3.00 | `_int_malloc` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 539.2 | 539.2 | 0.0 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 497.0 | 498.8 | 1.8 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 496.8 | 479.5 | -17.4 | 99.37 | `__aarch64_ldadd8_relax` |
| 480.0 | 480.0 | 0.0 | 9.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 467.2 | 296.7 | -170.5 | 9.96 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.2 | 417.2 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 398.4 | 224.0 | -174.4 | 7.74 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 377.3 | 290.2 | -87.0 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 352.2 | 352.2 | -0.0 | 11.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 348.8 | 348.8 | -0.0 | 69.76 | `__aarch64_cas4_acq` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 337.9 | 337.9 | -0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 318.8 | 318.8 | -0.0 | 63.76 | `__aarch64_swp4_rel` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 274.0 | 274.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.2 | 255.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 250.9 | 251.0 | 0.1 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 238.4 | 238.4 | -0.0 | 47.68 | `__aarch64_ldadd8_rel` |
| 237.6 | 237.6 | 0.0 | 2.00 | `malloc_consolidate` |
| 229.0 | 229.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 215.2 | 197.8 | -17.4 | 43.04 | `__aarch64_cas8_acq_rel` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.3 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 194.4 | 194.4 | 0.0 | 8.00 | `counts::main` |
| 190.0 | 190.0 | 0.0 | 2.00 | `<bacnet_objects::command_source::tracking::ValueSourceTracking>::command::<f32>` |
| 190.0 | 187.3 | -2.7 | 1.99 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 188.0 | 188.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 180.0 | 180.0 | 0.0 | 2.00 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 398.4 | 224.0 | -174.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 467.2 | 296.7 | -170.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.0 | 0.0 | -117.0 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.3 | 0.0 | -87.3 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 377.3 | 290.2 | -87.0 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 76.3 | 0.0 | -76.3 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.0 | 72.0 | 72.0 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.0 | 63.0 | 63.0 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closure#0}::{cl` |
| 107.6 | 59.0 | -48.6 | `tokio::runtime::task::waker::drop_waker` |
| 163.7 | 115.1 | -48.6 | `tokio::runtime::task::waker::clone_waker` |
| 92.6 | 62.0 | -30.6 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 59.8 | 42.4 | -17.4 | `__aarch64_cas8_acq` |
| 215.2 | 197.8 | -17.4 | `__aarch64_cas8_acq_rel` |
| 496.8 | 479.5 | -17.4 | `__aarch64_ldadd8_relax` |
| 83.5 | 66.1 | -17.4 | `__aarch64_ldadd8_acq_rel` |
| 865.3 | 878.8 | 13.5 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 190.0 | 187.3 | -2.7 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 497.0 | 498.8 | 1.8 | `<tokio::runtime::time::Driver>::park_internal` |
| 92.9 | 91.9 | -1.0 | `counts::main::{closure#0}` |
| 545.2 | 545.8 | 0.6 | `_int_malloc` |
| 101.0 | 101.4 | 0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 137.3 | 136.9 | -0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 96.8 | 96.5 | -0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 894.5 | 894.8 | 0.3 | `<std::sys::pal::unix::time::Timespec>::now` |
| 82.9 | 83.1 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 109.7 | 109.9 | 0.2 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 697.6 | 697.7 | 0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 34.4 | 34.5 | 0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 250.9 | 251.0 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 51.4 | 51.3 | -0.0 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 954.0 | 954.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 835.9 | 837.4 | 1.5 | `library/core/src/ptr/mod.rs:847` |
| 719.5 | 691.7 | -27.8 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 507.3 | 479.4 | -27.8 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 412.1 | 401.7 | -10.4 | `library/core/src/sync/atomic.rs:4029` |
| 374.8 | 374.8 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 288.0 | 288.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 285.8 | 285.8 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.0 | 283.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 256.1 | 250.8 | -5.2 | `library/core/src/option.rs:743` |
| 255.1 | 255.1 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 249.4 | 220.5 | -28.9 | `library/core/src/cell.rs:558` |
| 247.0 | 247.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 243.9 | 221.5 | -22.4 | `library/core/src/mem/mod.rs:976` |
| 220.0 | 220.0 | 0.0 | `library/core/src/result.rs:2177` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 189.0 | 189.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 179.9 | 172.9 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.1 | 156.7 | -17.4 | `library/core/src/sync/atomic.rs:4104` |
| 164.3 | 162.3 | -2.0 | `library/core/src/num/uint_macros.rs:2652` |
| 153.4 | 148.2 | -5.2 | `library/core/src/sync/atomic.rs:3995` |
| 152.3 | 152.3 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 150.0 | 150.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 146.9 | 133.4 | -13.4 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 144.0 | 144.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 139.0 | 139.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 130.0 | 130.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.8 | 119.9 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 126.8 | 115.1 | -11.7 | `library/core/src/cell.rs:2437` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 126.0 | 126.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 121.0 | 121.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 120.5 | 120.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 120.0 | 120.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 120.0 | 120.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 120.0 | 120.0 | 0.0 | `???:0` |
| 118.0 | 118.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 107.0 | 107.0 | -0.0 | `library/core/src/result.rs:0` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:171` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
| 107.0 | 107.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 105.0 | 105.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 105.0 | 105.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 102.0 | 102.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 102.0 | 104.3 | 2.3 | `library/core/src/ptr/non_null.rs:454` |
| 102.0 | 102.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 99.6 | 64.8 | -34.8 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 96.0 | 96.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 96.0 | 96.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
