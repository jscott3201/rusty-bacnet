# rp / ct: self instructions per request

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,538.0 | 1,538.0 | 0.0 | 22.00 | `_int_free` |
| 1,112.0 | 1,112.0 | 0.0 | 59.00 | `__GI_memcpy` |
| 1,026.0 | 1,026.0 | 0.0 | 19.00 | `malloc` |
| 941.0 | 941.0 | 0.0 | 22.00 | `free` |
| 804.4 | 804.4 | -0.0 | 17.01 | `<std::sys::pal::unix::time::Timespec>::now` |
| 798.0 | 798.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 698.2 | 698.2 | -0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 696.0 | 696.0 | 0.0 | 10.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 688.0 | 688.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>` |
| 549.6 | 549.6 | -0.1 | 9.01 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 545.0 | 545.0 | 0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 514.0 | 514.0 | 0.0 | 12.00 | `bacnet_transport::udp_metadata::unix::recv` |
| 506.3 | 506.2 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 458.6 | 458.6 | 0.0 | 91.73 | `__aarch64_ldadd8_relax` |
| 447.7 | 448.3 | 0.5 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 433.0 | 433.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 392.0 | 392.0 | 0.0 | 4.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 384.2 | 384.2 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 378.0 | 378.0 | 0.0 | 6.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 374.0 | 374.5 | 0.5 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 336.2 | 336.1 | -0.0 | 8.00 | `<tokio::sync::mpsc::chan::Rx<bacnet_transport::port::ReceivedNpdu, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 328.8 | 328.8 | -0.0 | 65.76 | `__aarch64_cas4_acq` |
| 320.0 | 320.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 304.0 | 304.0 | 0.0 | 3.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 298.8 | 298.8 | -0.0 | 59.76 | `__aarch64_swp4_rel` |
| 289.7 | 289.9 | 0.3 | 7.81 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 281.8 | 281.7 | -0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 278.0 | 278.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 268.2 | 268.2 | -0.0 | 2.00 | `<tokio::runtime::scheduler::current_thread::Context>::run_task` |
| 268.0 | 268.0 | 0.0 | 2.00 | `<bacnet_bip_core::node::config::Link>::receive::<bacnet_transport::bip::node::HostPolicy>` |
| 262.0 | 262.0 | 0.0 | 9.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 262.0 | 262.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 258.0 | 258.0 | 0.0 | 2.00 | `bacnet_network::layer::receive::admit` |
| 246.0 | 246.0 | 0.0 | 4.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 240.2 | 240.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule::{closure#0}` |
| 235.1 | 235.1 | -0.0 | 47.02 | `__aarch64_ldadd8_rel` |
| 226.2 | 226.3 | 0.1 | 31.50 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 221.0 | 221.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 215.3 | 215.3 | 0.0 | 43.06 | `__aarch64_cas8_acq_rel` |
| 208.2 | 208.2 | -0.1 | 4.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 203.0 | 203.0 | 0.0 | 4.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 199.2 | 199.2 | -0.0 | 4.00 | `<tokio::runtime::task::state::State>::transition_to_idle` |
| 195.0 | 195.0 | 0.0 | 4.00 | `bacnet_device_core::read::property::read_property_ack::<bacnet_server::device_view::DeviceReadContext, bacnet_server::server::requests::confirmed_response::read` |
| 192.1 | 192.1 | -0.0 | 6.00 | `<std::thread::local::LocalKey<tokio::runtime::context::Context>>::with::<tokio::runtime::context::thread_rng_n::{closure#0}, u32>` |
| 192.0 | 192.0 | 0.0 | 3.00 | `<core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHasher> as core::hash::BuildHasher>::hash_one::<&bacnet_server::server::request_peer::CanonicalReques` |
| 190.2 | 190.2 | -0.0 | 5.00 | `<tokio::runtime::task::state::State>::transition_to_running` |
| 189.0 | 189.0 | 0.0 | 3.00 | `<mio::poll::Poll>::poll` |
| 187.0 | 187.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 186.0 | 186.0 | 0.0 | 3.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 184.0 | 184.0 | 0.0 | 1.00 | `<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>::admit` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 176.2 | 176.2 | -0.0 | 4.01 | `<tokio::runtime::task::state::State>::transition_to_notified_by_val` |
| 175.9 | 175.9 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 174.2 | 174.2 | -0.0 | 8.00 | `<tokio::runtime::scheduler::current_thread::Core>::next_task` |
| 173.0 | 173.0 | 0.0 | 5.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 172.7 | 172.7 | -0.0 | 8.00 | `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closur` |
| 171.1 | 171.1 | -0.0 | 9.01 | `<std::time::Instant>::duration_since` |
| 170.1 | 170.1 | -0.0 | 17.01 | `clock_gettime@@GLIBC_2.17` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 447.7 | 448.3 | 0.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 139.6 | 139.1 | -0.5 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 374.0 | 374.5 | 0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 77.5 | 77.8 | 0.3 | `<tokio::time::interval::Interval>::poll_tick` |
| 289.7 | 289.9 | 0.3 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 107.9 | 108.1 | 0.2 | `tokio::runtime::task::waker::drop_waker` |
| 111.9 | 112.1 | 0.2 | `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv` |
| 163.9 | 164.1 | 0.2 | `tokio::runtime::task::waker::clone_waker` |
| 92.9 | 93.1 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 25.5 | 25.4 | -0.1 | `<tokio::sync::notify::Notified as core::future::future::Future>::poll` |
| 226.2 | 226.3 | 0.1 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 83.5 | 83.6 | 0.1 | `__aarch64_ldadd8_acq_rel` |
| 208.2 | 208.2 | -0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 56.1 | 56.0 | -0.1 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 46.7 | 46.6 | -0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 48.1 | 48.0 | -0.1 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 549.6 | 549.6 | -0.1 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 114.6 | 114.5 | -0.1 | `<tokio::runtime::time::wheel::Wheel>::poll` |
| 59.9 | 59.9 | 0.0 | `__aarch64_cas8_acq` |
| 0.5 | 0.5 | -0.0 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::lift::LiftFault>` |
| 109.9 | 110.0 | 0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 235.1 | 235.1 | -0.0 | `__aarch64_ldadd8_rel` |
| 0.4 | 0.3 | -0.0 | `<bacnet_objects::database::ObjectDatabase>::poll_trend_logs` |
| 804.4 | 804.4 | -0.0 | `<std::sys::pal::unix::time::Timespec>::now` |
| 328.8 | 328.8 | -0.0 | `__aarch64_cas4_acq` |
| 298.8 | 298.8 | -0.0 | `__aarch64_swp4_rel` |
| 137.0 | 137.0 | 0.0 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 215.3 | 215.3 | 0.0 | `__aarch64_cas8_acq_rel` |
| 55.3 | 55.3 | -0.0 | `<tokio::runtime::time::handle::Handle>::reregister` |
| 49.3 | 49.2 | -0.0 | `<tokio::runtime::time::wheel::Wheel>::insert` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 1,060.0 | 1,060.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 748.4 | 748.4 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 686.3 | 686.4 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 487.4 | 487.5 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 453.3 | 453.3 | 0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 369.2 | 369.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 362.0 | 361.9 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 306.2 | 306.2 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 289.5 | 289.5 | 0.0 | `library/core/src/cell.rs:558` |
| 283.2 | 283.2 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.6 | 267.7 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 267.1 | 267.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 264.0 | 264.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 231.1 | 231.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 229.0 | 229.0 | 0.0 | `library/core/src/result.rs:2177` |
| 218.0 | 218.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 178.0 | 178.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 173.2 | 173.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 171.6 | 171.6 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 171.0 | 171.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 164.1 | 164.1 | 0.0 | `library/core/src/option.rs:743` |
| 158.0 | 158.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 152.3 | 152.3 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 145.5 | 145.5 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 141.1 | 141.1 | 0.0 | `library/core/src/cell.rs:2437` |
| 133.8 | 133.8 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 126.0 | 126.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 121.9 | 121.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 112.0 | 112.0 | 0.0 | `library/core/src/result.rs:0` |
| 112.0 | 112.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 111.5 | 111.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 110.0 | 110.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 104.1 | 104.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 103.0 | 103.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 101.3 | 101.3 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 95.0 | 95.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 92.0 | 92.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 89.8 | 89.8 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 89.1 | 89.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 89.0 | 89.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 85.8 | 85.8 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
| 85.2 | 85.2 | -0.0 | `crates/bacnet-server/src/server/lifecycle.rs:198` |
| 81.1 | 81.1 | -0.0 | `library/core/src/option.rs:2439` |
| 80.0 | 80.0 | -0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 80.0 | 80.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:31` |
