# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,529.1 | 1,529.1 | 0.0 | 22.00 | `_int_free` |
| 1,026.0 | 1,026.1 | 0.0 | 19.00 | `malloc` |
| 1,003.1 | 1,003.1 | 0.0 | 22.00 | `free` |
| 947.0 | 0.0 | -947.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 913.9 | 912.3 | -1.6 | 13.14 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.3 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 736.0 | 848.0 | 112.0 | 39.00 | `__GI_memcpy` |
| 670.3 | 686.3 | 16.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 602.7 | 701.6 | 98.8 | 10.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 555.0 | 555.0 | 0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 473.1 | 473.1 | -0.0 | 91.00 | `__aarch64_cas4_acq` |
| 468.6 | 468.0 | -0.6 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 464.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 457.0 | 456.9 | -0.1 | 91.40 | `__aarch64_ldadd8_relax` |
| 425.1 | 424.1 | -1.0 | 1.31 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 409.3 | 409.3 | 0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 378.7 | 378.0 | -0.7 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 374.3 | 373.5 | -0.8 | 9.46 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 364.3 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 356.9 | 356.9 | -0.0 | 11.15 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.5 | 346.5 | -0.0 | 68.90 | `__aarch64_swp4_rel` |
| 338.0 | 338.0 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.1 | 319.1 | -0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 280.4 | 280.4 | 0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 276.0 | 276.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.2 | 274.2 | -0.0 | 2.00 | `_int_malloc` |
| 263.0 | 286.0 | 23.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.3 | 250.2 | -0.1 | 50.05 | `__aarch64_cas8_acq_rel` |
| 248.0 | 267.9 | 20.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 243.6 | 243.6 | 0.0 | 6.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 233.4 | 233.4 | -0.1 | 46.68 | `__aarch64_ldadd8_rel` |
| 232.0 | 232.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 231.0 | 284.0 | 53.0 | 2.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 230.0 | 230.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 220.0 | 220.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 215.0 | 216.0 | 1.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 211.1 | 211.8 | 0.7 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 210.0 | 210.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.2 | 204.1 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 196.4 | 194.6 | -1.8 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.1 | 193.2 | 0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_server::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_server::cov::identity::CovRecipien` |
| 177.0 | 177.1 | 0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 947.0 | 0.0 | -947.0 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 65.0 | 189.0 | 124.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 154.0 | 41.0 | -113.0 | `<bacnet_server::server::confirmed_request_tracker::ConfirmedRequestTracker>::begin` |
| 736.0 | 848.0 | 112.0 | `__GI_memcpy` |
| 602.7 | 701.6 | 98.8 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 0.0 | 59.0 | 59.0 | `<bacnet_endpoint_core::coordinator::CanonicalPeer as core::clone::Clone>::clone` |
| 231.0 | 284.0 | 53.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 263.0 | 286.0 | 23.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 87.0 | 109.0 | 22.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |
| 248.0 | 267.9 | 20.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 670.3 | 686.3 | 16.0 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 87.0 | 103.0 | 16.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 50.1 | 47.5 | -2.6 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 126.0 | 128.5 | 2.5 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 196.4 | 194.6 | -1.8 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 913.9 | 912.3 | -1.6 | `<std::sys::pal::unix::time::Timespec>::now` |
| 425.1 | 424.1 | -1.0 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 121.0 | 120.0 | -1.0 | `<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>::admit` |
| 91.0 | 92.0 | 1.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 215.0 | 216.0 | 1.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 374.3 | 373.5 | -0.8 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.7 | 378.0 | -0.7 | `<tokio::runtime::time::Driver>::park_internal` |
| 211.1 | 211.8 | 0.7 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 468.6 | 468.0 | -0.6 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 137.2 | 136.6 | -0.6 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 95.7 | 95.5 | -0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 87.4 | 87.6 | 0.2 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 76.4 | 76.6 | 0.2 | `<tokio::time::interval::Interval>::poll_tick` |
| 51.4 | 51.2 | -0.1 | `tokio::runtime::park::clone` |
| 44.0 | 43.9 | -0.1 | `tokio::runtime::park::drop_waker` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 711.6 | 711.5 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 697.3 | 697.3 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 660.0 | 797.0 | 137.0 | `library/core/src/ptr/mod.rs:573` |
| 552.2 | 552.1 | -0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 384.6 | 382.6 | -2.1 | `library/core/src/sync/atomic.rs:3994` |
| 370.2 | 366.1 | -4.0 | `library/core/src/sync/atomic.rs:4029` |
| 294.4 | 294.4 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 293.4 | 293.4 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 291.6 | 291.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 264.0 | 264.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 255.6 | 255.6 | -0.0 | `library/std/src/panicking.rs:465` |
| 237.2 | 237.2 | -0.0 | `library/core/src/cell.rs:558` |
| 226.5 | 226.5 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 190.0 | 13.0 | -177.0 | `library/core/src/slice/iter/macros.rs:180` |
| 177.9 | 177.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.3 | 174.2 | -0.1 | `library/core/src/sync/atomic.rs:4104` |
| 171.0 | 171.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 162.2 | 162.2 | -0.1 | `library/core/src/sync/atomic.rs:3995` |
| 160.4 | 160.4 | -0.0 | `library/core/src/option.rs:743` |
| 157.3 | 157.4 | 0.1 | `library/core/src/ptr/non_null.rs:454` |
| 156.6 | 156.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 156.6 | 156.6 | 0.0 | `???:0` |
| 155.0 | 154.9 | -0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 148.6 | 147.5 | -1.0 | `library/core/src/cell.rs:2437` |
| 138.1 | 138.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.0 | 138.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 130.8 | 130.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 79.0 | -47.0 | `library/core/src/ptr/mod.rs:1963` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 112.0 | 0.0 | -112.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 110.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 110.0 | 0.0 | `library/core/src/result.rs:2177` |
| 107.0 | 107.0 | -0.0 | `library/core/src/result.rs:0` |
| 103.0 | 103.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 100.2 | 100.2 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 99.9 | 99.7 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 95.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.1 | 94.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 85.0 | 85.0 | 0.0 | `./malloc/./malloc/arena.c:147` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.1 | 81.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 81.0 | 81.0 | -0.0 | `library/core/src/option.rs:2439` |
