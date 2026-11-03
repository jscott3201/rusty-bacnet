# rp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,538.0 | 1,538.0 | 0.0 | 22.00 | `_int_free` |
| 1,026.0 | 1,026.0 | 0.0 | 19.00 | `malloc` |
| 996.7 | 997.0 | 0.3 | 16.67 | `<std::sys::pal::unix::time::Timespec>::now` |
| 964.0 | 964.0 | 0.0 | 22.00 | `free` |
| 947.0 | 0.0 | -947.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 865.2 | 865.3 | 0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 736.0 | 848.0 | 112.0 | 39.00 | `__GI_memcpy` |
| 670.2 | 686.2 | 16.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 602.5 | 701.1 | 98.7 | 10.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 560.0 | 560.0 | 0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 497.7 | 500.0 | 2.3 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 467.6 | 467.9 | 0.4 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 464.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 456.9 | 456.9 | 0.1 | 91.37 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 409.2 | 409.2 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 384.2 | 384.2 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 372.5 | 372.5 | -0.0 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.3 | 364.2 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 337.8 | 337.9 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.7 | 328.7 | 0.0 | 65.75 | `__aarch64_cas4_acq` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 298.7 | 298.7 | 0.0 | 59.75 | `__aarch64_swp4_rel` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.7 | 284.7 | 0.1 | 7.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 278.0 | 278.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 276.0 | 276.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 263.0 | 286.0 | 23.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 255.2 | 255.2 | 0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 248.0 | 268.0 | 20.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 233.4 | 233.4 | 0.0 | 46.67 | `__aarch64_ldadd8_rel` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 231.0 | 284.0 | 53.0 | 2.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 230.0 | 230.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 215.2 | 215.3 | 0.0 | 43.05 | `__aarch64_cas8_acq_rel` |
| 215.0 | 216.0 | 1.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 210.0 | 210.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 194.3 | 194.4 | 0.0 | 8.00 | `counts::main` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 188.8 | 185.4 | -3.4 | 1.98 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_server::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_server::cov::identity::CovRecipien` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 170.2 | 170.2 | 0.0 | 4.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 170.1 | 170.1 | -0.0 | 17.01 | `clock_gettime@@GLIBC_2.17` |
| 163.8 | 164.0 | 0.2 | 11.70 | `tokio::runtime::task::waker::clone_waker` |
| 162.4 | 162.3 | -0.1 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 947.0 | 0.0 | -947.0 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 65.0 | 189.0 | 124.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 154.0 | 41.0 | -113.0 | `<bacnet_server::server::confirmed_request_tracker::ConfirmedRequestTracker>::begin` |
| 736.0 | 848.0 | 112.0 | `__GI_memcpy` |
| 602.5 | 701.1 | 98.7 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 0.0 | 59.0 | 59.0 | `<bacnet_endpoint_core::coordinator::CanonicalPeer as core::clone::Clone>::clone` |
| 231.0 | 284.0 | 53.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 263.0 | 286.0 | 23.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 87.0 | 109.0 | 22.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |
| 248.0 | 268.0 | 20.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 87.0 | 103.0 | 16.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 670.2 | 686.2 | 16.0 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 188.8 | 185.4 | -3.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 497.7 | 500.0 | 2.3 | `<tokio::runtime::time::Driver>::park_internal` |
| 121.0 | 120.0 | -1.0 | `<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>::admit` |
| 215.0 | 216.0 | 1.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 91.0 | 92.0 | 1.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 87.2 | 87.8 | 0.6 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 117.1 | 117.7 | 0.6 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 101.0 | 101.4 | 0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 137.1 | 136.7 | -0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 467.6 | 467.9 | 0.4 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 996.7 | 997.0 | 0.3 | `<std::sys::pal::unix::time::Timespec>::now` |
| 109.8 | 110.0 | 0.2 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 82.7 | 83.0 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 95.7 | 95.5 | -0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 107.7 | 108.0 | 0.2 | `tokio::runtime::task::waker::drop_waker` |
| 163.8 | 164.0 | 0.2 | `tokio::runtime::task::waker::clone_waker` |
| 34.4 | 34.5 | 0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 865.2 | 865.3 | 0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 714.9 | 714.9 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 683.5 | 683.6 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 660.0 | 797.0 | 137.0 | `library/core/src/ptr/mod.rs:573` |
| 487.3 | 487.3 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 367.1 | 363.1 | -4.0 | `library/core/src/sync/atomic.rs:4029` |
| 360.8 | 358.8 | -2.0 | `library/core/src/sync/atomic.rs:3994` |
| 289.1 | 289.1 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 268.7 | 268.7 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.0 | 267.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 264.0 | 264.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 251.9 | 251.9 | 0.0 | `library/core/src/cell.rs:558` |
| 246.0 | 246.0 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 231.0 | 231.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 184.0 | 7.0 | -177.0 | `library/core/src/slice/iter/macros.rs:180` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.1 | 174.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 171.0 | 171.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 170.9 | 170.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 163.1 | 163.1 | 0.0 | `library/core/src/option.rs:743` |
| 146.9 | 146.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 145.4 | 145.4 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 143.2 | 143.3 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 138.0 | 138.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 130.8 | 129.9 | -1.0 | `library/core/src/cell.rs:2437` |
| 129.0 | 82.0 | -47.0 | `library/core/src/ptr/mod.rs:1963` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 121.8 | 121.8 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 112.5 | 112.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 112.0 | 0.0 | -112.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 110.0 | 0.0 | `library/core/src/result.rs:2177` |
| 107.0 | 107.0 | -0.0 | `library/core/src/result.rs:0` |
| 103.0 | 103.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 102.0 | 102.0 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 99.7 | 99.7 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 95.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 92.0 | 92.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 91.0 | 91.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 88.0 | 88.0 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 82.0 | 82.0 | -0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 82.0 | 82.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 81.0 | 81.0 | -0.0 | `library/core/src/option.rs:2439` |
| 81.0 | 81.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
