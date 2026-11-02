# rp / mt: self instructions per request

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,538.4 | 1,542.9 | 4.5 | 44.00 | `_int_free` |
| 1,112.0 | 1,112.0 | 0.0 | 59.00 | `__GI_memcpy` |
| 1,026.1 | 1,026.0 | -0.0 | 19.00 | `malloc` |
| 980.1 | 980.0 | -0.0 | 29.00 | `free` |
| 830.4 | 830.7 | 0.3 | 17.27 | `<std::sys::pal::unix::time::Timespec>::now` |
| 798.0 | 798.0 | 0.0 | 16.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 698.4 | 698.4 | 0.0 | 14.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 696.0 | 696.0 | 0.0 | 14.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 688.0 | 688.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>` |
| 545.0 | 545.0 | -0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 514.0 | 514.0 | 0.0 | 18.00 | `bacnet_transport::udp_metadata::unix::recv` |
| 506.4 | 506.4 | 0.0 | 20.02 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 473.2 | 473.2 | 0.1 | 91.02 | `__aarch64_cas4_acq` |
| 457.0 | 457.0 | 0.1 | 91.39 | `__aarch64_ldadd8_relax` |
| 448.2 | 448.2 | 0.0 | 16.96 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 433.0 | 433.0 | -0.0 | 9.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 8.00 | `bacnet_encoding::apdu::decode_apdu` |
| 392.0 | 392.0 | -0.0 | 8.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 378.0 | 378.0 | 0.0 | 10.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 376.2 | 376.3 | 0.1 | 12.31 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 375.3 | 375.5 | 0.2 | 11.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 357.2 | 357.2 | 0.0 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.6 | 346.7 | 0.0 | 68.92 | `__aarch64_swp4_rel` |
| 336.3 | 336.3 | 0.0 | 10.01 | `<tokio::sync::mpsc::chan::Rx<bacnet_transport::port::ReceivedNpdu, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 320.0 | 320.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 304.0 | 304.0 | 0.0 | 9.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 289.8 | 290.3 | 0.4 | 11.82 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 285.8 | 291.3 | 5.5 | 6.29 | `_int_malloc` |
| 285.5 | 285.5 | 0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 281.9 | 281.9 | -0.0 | 10.02 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 278.0 | 278.0 | 0.0 | 6.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 268.0 | 268.0 | 0.0 | 4.00 | `<bacnet_bip_core::node::config::Link>::receive::<bacnet_transport::bip::node::HostPolicy>` |
| 262.0 | 262.0 | 0.0 | 8.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 258.0 | 258.0 | 0.0 | 4.00 | `bacnet_network::layer::receive::admit` |
| 250.4 | 250.4 | 0.0 | 50.07 | `__aarch64_cas8_acq_rel` |
| 246.0 | 246.0 | 0.0 | 4.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 241.1 | 241.1 | 0.0 | 12.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 233.4 | 233.4 | -0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 226.5 | 226.4 | -0.1 | 31.52 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 221.0 | 221.0 | -0.0 | 7.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 218.5 | 218.5 | -0.1 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 209.6 | 209.8 | 0.2 | 2.13 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 208.3 | 208.3 | 0.0 | 4.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 204.3 | 204.2 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 203.0 | 203.0 | 0.0 | 4.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 199.4 | 199.4 | 0.0 | 8.01 | `<tokio::runtime::task::state::State>::transition_to_idle` |
| 195.0 | 195.0 | 0.0 | 5.00 | `bacnet_device_core::read::property::read_property_ack::<bacnet_server::device_view::DeviceReadContext, bacnet_server::server::requests::confirmed_response::read` |
| 193.2 | 193.2 | 0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.2 | 192.2 | 0.0 | 12.01 | `<std::thread::local::LocalKey<tokio::runtime::context::Context>>::with::<tokio::runtime::context::thread_rng_n::{closure#0}, u32>` |
| 192.0 | 192.0 | 0.0 | 3.00 | `<core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHasher> as core::hash::BuildHasher>::hash_one::<&bacnet_server::server::request_peer::CanonicalReques` |
| 190.3 | 190.3 | 0.0 | 10.01 | `<tokio::runtime::task::state::State>::transition_to_running` |
| 187.0 | 187.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 186.0 | 186.0 | 0.0 | 6.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 184.0 | 184.0 | 0.0 | 4.00 | `<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>::admit` |
| 183.0 | 183.0 | 0.0 | 9.00 | `bacnet_encoding::tags::decode_tag` |
| 177.1 | 177.1 | 0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 176.3 | 176.3 | -0.0 | 12.02 | `<tokio::runtime::task::state::State>::transition_to_notified_by_val` |
| 175.9 | 175.9 | 0.0 | 6.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 173.0 | 173.0 | -0.0 | 5.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 172.7 | 172.7 | -0.0 | 9.00 | `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closur` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 285.8 | 291.3 | 5.5 | `_int_malloc` |
| 1,538.4 | 1,542.9 | 4.5 | `_int_free` |
| 25.0 | 25.8 | 0.8 | `__aarch64_cas8_rel` |
| 79.7 | 80.4 | 0.7 | `__aarch64_cas8_acq` |
| 112.5 | 112.0 | -0.5 | `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv` |
| 88.3 | 88.7 | 0.5 | `<core::future::poll_fn::PollFn<<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}::{closure#0}> as core::futur` |
| 289.8 | 290.3 | 0.4 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 93.4 | 93.0 | -0.4 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 830.4 | 830.7 | 0.3 | `<std::sys::pal::unix::time::Timespec>::now` |
| 375.3 | 375.5 | 0.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 35.0 | 35.2 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 137.1 | 136.9 | -0.2 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 209.6 | 209.8 | 0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 226.5 | 226.4 | -0.1 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 46.6 | 46.7 | 0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 376.2 | 376.3 | 0.1 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 137.0 | 136.9 | -0.1 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 107.9 | 108.0 | 0.1 | `tokio::runtime::task::waker::drop_waker` |
| 457.0 | 457.0 | 0.1 | `__aarch64_ldadd8_relax` |
| 164.0 | 164.1 | 0.1 | `tokio::runtime::task::waker::clone_waker` |
| 218.5 | 218.5 | -0.1 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 0.9 | 1.0 | 0.1 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 473.2 | 473.2 | 0.1 | `__aarch64_cas4_acq` |
| 0.6 | 0.6 | 0.0 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::is_parked` |
| 346.6 | 346.7 | 0.0 | `__aarch64_swp4_rel` |
| 84.8 | 84.7 | -0.0 | `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 101.3 | 101.3 | 0.0 | `<core::future::poll_fn::PollFn<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}::{closure#0}` |
| 152.3 | 152.4 | 0.0 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run` |
| 448.2 | 448.2 | 0.0 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 113.4 | 113.4 | 0.0 | `__aarch64_ldadd8_acq_rel` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 1,060.1 | 1,060.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 737.8 | 737.8 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 711.6 | 711.7 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 552.3 | 552.4 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 429.5 | 429.5 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 393.7 | 393.7 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 372.2 | 372.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 310.8 | 310.9 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 309.9 | 310.0 | 0.1 | `library/core/src/sync/atomic.rs:4083` |
| 291.7 | 291.7 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 264.0 | 264.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 263.8 | 263.8 | -0.0 | `library/core/src/cell.rs:558` |
| 255.7 | 255.7 | 0.0 | `library/std/src/panicking.rs:465` |
| 242.5 | 242.5 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 229.0 | 229.0 | -0.0 | `library/core/src/result.rs:2177` |
| 218.0 | 218.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 186.1 | 186.1 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 184.0 | 184.0 | 0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 182.0 | 183.6 | 1.6 | `???:0` |
| 177.9 | 177.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 173.3 | 173.3 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 171.0 | 171.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 168.7 | 168.7 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 164.3 | 164.3 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 161.6 | 161.6 | 0.0 | `library/core/src/option.rs:743` |
| 158.0 | 158.0 | -0.0 | `library/core/src/cell.rs:2437` |
| 158.0 | 158.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 138.1 | 138.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 132.0 | 132.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 132.0 | 132.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 129.8 | 129.8 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 123.0 | 123.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 114.0 | 114.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 114.0 | 114.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 114.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 112.0 | 112.0 | -0.0 | `library/core/src/result.rs:0` |
| 112.0 | 112.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 111.2 | 111.2 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 110.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 109.4 | 109.4 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 104.1 | 104.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 103.0 | 103.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 95.0 | 95.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 92.1 | 92.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 89.8 | 89.9 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 85.9 | 85.8 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
| 85.3 | 85.3 | 0.0 | `crates/bacnet-server/src/server/lifecycle.rs:198` |
| 85.0 | 85.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 85.0 | 85.0 | -0.0 | `./malloc/./malloc/arena.c:147` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.1 | 81.1 | -0.0 | `library/core/src/option.rs:2439` |
