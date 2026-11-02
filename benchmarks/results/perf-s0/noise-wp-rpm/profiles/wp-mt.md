# wp / mt: self instructions per request

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,750.3 | 1,750.1 | -0.2 | 51.00 | `_int_free` |
| 1,699.0 | 1,699.0 | 0.0 | 53.00 | `__GI_memcpy` |
| 1,279.5 | 1,279.1 | -0.4 | 34.00 | `free` |
| 1,250.1 | 1,250.1 | 0.0 | 23.00 | `malloc` |
| 1,118.0 | 1,118.0 | 0.0 | 13.00 | `<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>` |
| 1,048.4 | 1,048.4 | 0.0 | 22.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 918.1 | 917.8 | -0.3 | 19.28 | `<std::sys::pal::unix::time::Timespec>::now` |
| 808.5 | 808.5 | 0.0 | 36.02 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 798.0 | 798.0 | 0.0 | 16.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 696.0 | 696.0 | 0.0 | 14.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 649.0 | 649.0 | -0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 29.00 | `bacnet_encoding::tags::decode_tag` |
| 568.0 | 568.0 | 0.0 | 28.06 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.7 | 546.7 | 0.0 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 514.0 | 514.0 | 0.0 | 18.00 | `bacnet_transport::udp_metadata::unix::recv` |
| 512.0 | 512.1 | 0.1 | 102.41 | `__aarch64_ldadd8_relax` |
| 505.3 | 505.3 | 0.0 | 97.04 | `__aarch64_cas4_acq` |
| 483.2 | 464.0 | -19.2 | 10.57 | `_int_malloc` |
| 476.0 | 476.0 | 0.0 | 9.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 448.1 | 448.6 | 0.5 | 16.95 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 392.0 | 392.0 | 0.0 | 8.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 381.0 | 381.0 | 0.0 | 9.00 | `bacnet_encoding::tags::extract_context_value` |
| 378.7 | 378.7 | 0.0 | 74.94 | `__aarch64_swp4_rel` |
| 376.5 | 376.5 | 0.0 | 12.32 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 375.3 | 376.0 | 0.6 | 11.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 372.0 | 372.0 | 0.0 | 7.00 | `bacnet_encoding::apdu::decode_apdu` |
| 357.3 | 357.3 | -0.0 | 11.17 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 336.3 | 336.3 | -0.0 | 10.01 | `<tokio::sync::mpsc::chan::Rx<bacnet_transport::port::ReceivedNpdu, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 322.0 | 322.0 | 0.0 | 6.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 320.0 | 320.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 290.2 | 290.4 | 0.2 | 11.83 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 285.6 | 285.6 | 0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 282.0 | 282.0 | -0.0 | 10.02 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 278.0 | 278.0 | 0.0 | 6.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 271.0 | 271.0 | 0.0 | 8.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 270.4 | 270.5 | 0.0 | 54.09 | `__aarch64_cas8_acq_rel` |
| 270.0 | 270.0 | 0.0 | 6.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 268.0 | 268.0 | 0.0 | 4.00 | `<bacnet_bip_core::node::config::Link>::receive::<bacnet_transport::bip::node::HostPolicy>` |
| 262.0 | 262.0 | 0.0 | 8.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 258.0 | 258.0 | 0.0 | 6.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 258.0 | 258.0 | 0.0 | 4.00 | `bacnet_network::layer::receive::admit` |
| 253.4 | 253.4 | -0.0 | 50.69 | `__aarch64_ldadd8_rel` |
| 246.0 | 246.0 | 0.0 | 4.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 241.1 | 241.1 | -0.0 | 12.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 226.5 | 226.5 | 0.1 | 31.52 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 225.1 | 225.1 | 0.0 | 9.00 | `<tokio::sync::batch_semaphore::Semaphore>::release` |
| 221.0 | 221.0 | -0.0 | 7.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 218.6 | 218.7 | 0.1 | 4.02 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 212.0 | 212.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 209.9 | 210.0 | 0.1 | 2.14 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 208.4 | 208.4 | -0.0 | 4.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 207.0 | 207.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 204.3 | 204.3 | 0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 199.4 | 199.4 | 0.0 | 8.02 | `<tokio::runtime::task::state::State>::transition_to_idle` |
| 198.0 | 198.0 | 0.0 | 6.00 | `bacnet_encoding::constructed::tagged::next_is_context` |
| 193.2 | 193.2 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.8 | 192.8 | -0.0 | 19.28 | `clock_gettime@@GLIBC_2.17` |
| 192.3 | 192.3 | -0.0 | 12.02 | `<std::thread::local::LocalKey<tokio::runtime::context::Context>>::with::<tokio::runtime::context::thread_rng_n::{closure#0}, u32>` |
| 192.0 | 192.0 | 0.0 | 6.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 192.0 | 192.0 | 0.0 | 3.00 | `<core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHasher> as core::hash::BuildHasher>::hash_one::<&bacnet_server::server::request_peer::CanonicalReques` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 483.2 | 464.0 | -19.2 | `_int_malloc` |
| 36.4 | 30.3 | -6.1 | `_int_free_maybe_consolidate.part.0` |
| 13.6 | 8.3 | -5.4 | `unlink_chunk.isra.0` |
| 141.3 | 139.3 | -1.9 | `malloc_consolidate` |
| 25.7 | 27.5 | 1.7 | `_int_free_create_chunk` |
| 375.3 | 376.0 | 0.6 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 152.4 | 153.0 | 0.6 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 5.4 | 4.9 | -0.5 | `alloc_perturb` |
| 448.1 | 448.6 | 0.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 27.6 | 28.0 | 0.4 | `core::ptr::drop_glue::<core::pin::Pin<alloc::boxed::Box<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTran` |
| 1,279.5 | 1,279.1 | -0.4 | `free` |
| 918.1 | 917.8 | -0.3 | `<std::sys::pal::unix::time::Timespec>::now` |
| 1,750.3 | 1,750.1 | -0.2 | `_int_free` |
| 108.0 | 108.2 | 0.2 | `tokio::runtime::task::waker::drop_waker` |
| 290.2 | 290.4 | 0.2 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 77.4 | 77.6 | 0.2 | `<tokio::time::interval::Interval>::poll_tick` |
| 164.1 | 164.3 | 0.2 | `tokio::runtime::task::waker::clone_waker` |
| 136.8 | 136.7 | -0.1 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 112.2 | 112.3 | 0.1 | `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv` |
| 41.6 | 41.5 | -0.1 | `_int_free_merge_chunk` |
| 218.6 | 218.7 | 0.1 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 49.9 | 49.8 | -0.1 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 209.9 | 210.0 | 0.1 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 226.5 | 226.5 | 0.1 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 93.2 | 93.3 | 0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 164.9 | 164.9 | -0.1 | `<tokio::runtime::time::Driver>::park_internal` |
| 1.1 | 1.1 | 0.1 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 512.0 | 512.1 | 0.1 | `__aarch64_ldadd8_relax` |
| 65.9 | 66.0 | 0.1 | `__aarch64_cas8_acq` |
| 110.3 | 110.3 | 0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 1,024.1 | 1,024.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 830.9 | 830.9 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 771.8 | 771.9 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 588.5 | 588.6 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 453.6 | 453.7 | 0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 430.2 | 430.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 418.8 | 418.8 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 355.0 | 355.0 | 0.0 | `library/core/src/result.rs:2177` |
| 347.0 | 347.0 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 330.0 | 330.1 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 330.0 | 330.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 312.0 | 312.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 307.8 | 307.8 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 281.9 | 281.9 | 0.1 | `library/core/src/cell.rs:558` |
| 271.7 | 271.8 | 0.0 | `library/std/src/panicking.rs:465` |
| 261.0 | 261.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 250.6 | 250.6 | 0.0 | `library/core/src/option.rs:743` |
| 248.6 | 248.6 | -0.0 | `???:0` |
| 248.6 | 248.6 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 222.0 | 222.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 192.9 | 193.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 192.2 | 192.2 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 189.3 | 189.4 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 182.0 | 182.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 180.7 | 180.8 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 176.1 | 176.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 168.3 | 168.4 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 161.1 | 161.1 | 0.0 | `library/core/src/cell.rs:2437` |
| 158.0 | 158.0 | 0.0 | `library/core/src/result.rs:0` |
| 158.0 | 158.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 156.0 | 156.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 154.1 | 154.1 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 151.4 | 151.4 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 151.0 | 151.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 147.1 | 147.2 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 137.8 | 137.9 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 130.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 129.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 125.0 | 125.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.1 | 121.1 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 117.0 | 117.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:1010` |
| 116.0 | 116.0 | 0.0 | `library/core/src/slice/iter/macros.rs:379` |
| 115.0 | 115.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 110.1 | 110.2 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 107.4 | 107.4 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 105.6 | 105.5 | -0.1 | `./malloc/./malloc/arena.c:147` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
