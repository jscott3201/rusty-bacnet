# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,529.1 | 1,529.1 | 0.0 | 22.00 | `_int_free` |
| 1,026.0 | 1,026.1 | 0.0 | 19.00 | `malloc` |
| 1,003.1 | 1,003.1 | 0.0 | 22.00 | `free` |
| 914.4 | 912.6 | -1.8 | 13.13 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.5 | 0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 848.0 | 848.0 | 0.0 | 46.00 | `__GI_memcpy` |
| 701.5 | 634.5 | -67.0 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 686.3 | 686.3 | -0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 555.0 | 555.0 | 0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 473.1 | 473.1 | 0.0 | 90.99 | `__aarch64_cas4_acq` |
| 467.6 | 468.3 | 0.7 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 464.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 456.9 | 457.0 | 0.1 | 91.38 | `__aarch64_ldadd8_relax` |
| 424.3 | 425.2 | 0.9 | 1.33 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | -0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 409.3 | 409.3 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 378.2 | 378.6 | 0.4 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 373.7 | 374.1 | 0.5 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.3 | 364.2 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 356.9 | 356.8 | -0.0 | 11.15 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.5 | 346.5 | 0.1 | 68.89 | `__aarch64_swp4_rel` |
| 338.0 | 338.0 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.0 | 319.0 | -0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 286.0 | 286.0 | 0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 284.0 | 284.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 280.4 | 280.4 | -0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 276.0 | 277.0 | 1.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.1 | 274.3 | 0.1 | 2.00 | `_int_malloc` |
| 268.0 | 268.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.2 | 250.2 | 0.0 | 50.04 | `__aarch64_cas8_acq_rel` |
| 243.6 | 243.6 | 0.1 | 6.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 233.4 | 233.4 | 0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 230.0 | 230.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 216.0 | 216.0 | -0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 211.4 | 211.1 | -0.3 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 210.0 | 210.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.1 | 204.2 | 0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 196.6 | 195.4 | -1.2 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.1 | 193.1 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_server::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_server::cov::identity::CovRecipien` |
| 177.0 | 177.0 | -0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 701.5 | 634.5 | -67.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 42.0 | 28.0 | -14.0 | `core::ptr::drop_glue::<bacnet_network::response_route::ResponseRoute>` |
| 50.6 | 48.6 | -2.0 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 914.4 | 912.6 | -1.8 | `<std::sys::pal::unix::time::Timespec>::now` |
| 127.3 | 125.9 | -1.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 196.6 | 195.4 | -1.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 276.0 | 277.0 | 1.0 | `bacnet_encoding::apdu::decode_apdu` |
| 424.3 | 425.2 | 0.9 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 467.6 | 468.3 | 0.7 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 373.7 | 374.1 | 0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 101.0 | 101.4 | 0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 378.2 | 378.6 | 0.4 | `<tokio::runtime::time::Driver>::park_internal` |
| 82.8 | 83.1 | 0.3 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 137.1 | 137.4 | 0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 211.4 | 211.1 | -0.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 107.8 | 108.0 | 0.2 | `tokio::runtime::task::waker::drop_waker` |
| 117.3 | 117.5 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.3 | 87.5 | 0.2 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 163.9 | 164.0 | 0.2 | `tokio::runtime::task::waker::clone_waker` |
| 865.4 | 865.5 | 0.2 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 274.1 | 274.3 | 0.1 | `_int_malloc` |
| 76.5 | 76.6 | 0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 364.3 | 364.2 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 95.7 | 95.8 | 0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 0.6 | 0.5 | -0.1 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 456.9 | 457.0 | 0.1 | `__aarch64_ldadd8_relax` |
| 113.3 | 113.3 | 0.1 | `__aarch64_ldadd8_acq_rel` |
| 346.5 | 346.5 | 0.1 | `__aarch64_swp4_rel` |
| 243.6 | 243.6 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 77.8 | 77.9 | 0.1 | `__aarch64_cas8_acq` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 797.0 | 860.0 | 63.0 | `library/core/src/ptr/mod.rs:573` |
| 711.4 | 711.5 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 697.3 | 675.3 | -22.0 | `library/core/src/ptr/mod.rs:847` |
| 552.1 | 552.2 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 382.6 | 382.6 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 366.1 | 366.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 294.4 | 294.4 | 0.1 | `library/core/src/sync/atomic.rs:4083` |
| 293.3 | 293.3 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 291.5 | 291.6 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 264.0 | 264.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 255.6 | 255.6 | 0.0 | `library/std/src/panicking.rs:465` |
| 237.2 | 237.2 | 0.0 | `library/core/src/cell.rs:558` |
| 226.5 | 224.5 | -2.0 | `library/core/src/mem/mod.rs:976` |
| 177.9 | 177.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.2 | 174.2 | 0.1 | `library/core/src/sync/atomic.rs:4104` |
| 171.0 | 171.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 162.3 | 162.2 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 160.4 | 160.4 | 0.0 | `library/core/src/option.rs:743` |
| 157.3 | 157.4 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 156.6 | 156.6 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 156.6 | 156.6 | -0.0 | `???:0` |
| 154.9 | 155.0 | 0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 147.6 | 147.6 | 0.0 | `library/core/src/cell.rs:2437` |
| 138.0 | 138.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.0 | 138.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 130.7 | 130.8 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 110.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 110.0 | 0.0 | `library/core/src/result.rs:2177` |
| 107.0 | 107.0 | 0.0 | `library/core/src/result.rs:0` |
| 103.0 | 103.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 100.2 | 99.2 | -1.0 | `library/core/src/mem/mod.rs:975` |
| 99.7 | 99.8 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 95.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.1 | 94.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 85.0 | 85.0 | 0.0 | `./malloc/./malloc/arena.c:147` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.1 | 81.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 81.1 | 81.1 | 0.0 | `library/core/src/option.rs:2439` |
| 81.0 | 81.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 80.0 | 80.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 79.5 | 79.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
