# wp / ct: self instructions per request

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,764.0 | 1,764.0 | 0.0 | 26.00 | `_int_free` |
| 1,699.0 | 1,699.0 | 0.0 | 53.00 | `__GI_memcpy` |
| 1,244.0 | 1,244.0 | -0.0 | 23.00 | `malloc` |
| 1,221.0 | 1,221.0 | 0.0 | 26.00 | `free` |
| 1,118.0 | 1,118.0 | 0.0 | 13.00 | `<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>` |
| 1,048.3 | 1,048.3 | -0.0 | 11.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 891.5 | 891.4 | -0.0 | 19.01 | `<std::sys::pal::unix::time::Timespec>::now` |
| 808.3 | 808.3 | -0.0 | 9.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 798.0 | 798.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 696.0 | 696.0 | 0.0 | 10.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 649.0 | 649.0 | 0.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.7 | 567.7 | -0.0 | 14.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 549.9 | 549.5 | -0.4 | 9.01 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 546.5 | 546.5 | 0.0 | 7.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 543.1 | 543.1 | 0.0 | 3.00 | `_int_malloc` |
| 514.0 | 514.0 | 0.0 | 12.00 | `bacnet_transport::udp_metadata::unix::recv` |
| 513.6 | 513.6 | 0.1 | 102.72 | `__aarch64_ldadd8_relax` |
| 476.0 | 476.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 447.4 | 447.6 | 0.2 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 392.0 | 392.0 | 0.0 | 4.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 384.3 | 384.3 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 381.0 | 381.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 373.9 | 374.4 | 0.5 | 9.42 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 372.0 | 372.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 348.8 | 348.8 | 0.0 | 69.76 | `__aarch64_cas4_acq` |
| 336.2 | 336.2 | -0.0 | 8.01 | `<tokio::sync::mpsc::chan::Rx<bacnet_transport::port::ReceivedNpdu, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 322.0 | 322.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 320.0 | 320.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 318.8 | 318.8 | 0.0 | 63.76 | `__aarch64_swp4_rel` |
| 289.8 | 289.9 | 0.1 | 7.81 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 281.8 | 281.8 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 278.0 | 278.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 271.0 | 271.0 | 0.0 | 3.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 270.0 | 270.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 268.3 | 268.3 | -0.0 | 2.00 | `<tokio::runtime::scheduler::current_thread::Context>::run_task` |
| 268.0 | 268.0 | 0.0 | 2.00 | `<bacnet_bip_core::node::config::Link>::receive::<bacnet_transport::bip::node::HostPolicy>` |
| 262.0 | 262.0 | 0.0 | 9.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 262.0 | 262.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 258.0 | 258.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 258.0 | 258.0 | 0.0 | 2.00 | `bacnet_network::layer::receive::admit` |
| 255.0 | 255.0 | -0.0 | 51.01 | `__aarch64_ldadd8_rel` |
| 246.0 | 246.0 | 0.0 | 4.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 240.3 | 240.3 | -0.0 | 5.01 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule::{closure#0}` |
| 237.4 | 237.4 | 0.0 | 2.00 | `malloc_consolidate` |
| 235.3 | 235.3 | -0.0 | 47.06 | `__aarch64_cas8_acq_rel` |
| 226.4 | 226.1 | -0.3 | 31.51 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 225.1 | 225.1 | -0.0 | 9.00 | `<tokio::sync::batch_semaphore::Semaphore>::release` |
| 221.0 | 221.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 212.0 | 212.0 | 0.0 | 6.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 208.4 | 208.2 | -0.1 | 4.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 207.0 | 207.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 199.3 | 199.3 | -0.0 | 4.01 | `<tokio::runtime::task::state::State>::transition_to_idle` |
| 198.0 | 198.0 | 0.0 | 6.00 | `bacnet_encoding::constructed::tagged::next_is_context` |
| 192.2 | 192.2 | -0.0 | 6.01 | `<std::thread::local::LocalKey<tokio::runtime::context::Context>>::with::<tokio::runtime::context::thread_rng_n::{closure#0}, u32>` |
| 192.0 | 192.0 | 0.0 | 3.00 | `<core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHasher> as core::hash::BuildHasher>::hash_one::<&bacnet_server::server::request_peer::CanonicalReques` |
| 192.0 | 192.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 190.2 | 190.2 | -0.0 | 5.01 | `<tokio::runtime::task::state::State>::transition_to_running` |
| 190.1 | 190.1 | -0.0 | 19.01 | `clock_gettime@@GLIBC_2.17` |
| 189.0 | 189.0 | 0.0 | 3.00 | `<mio::poll::Poll>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 110.1 | 109.6 | -0.5 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 373.9 | 374.4 | 0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 84.8 | 84.4 | -0.4 | `<tokio::sync::mpsc::chan::Rx<bacnet_network::layer::ReceivedNetworkControl, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 549.9 | 549.5 | -0.4 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 226.4 | 226.1 | -0.3 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 88.5 | 88.7 | 0.2 | `<core::future::poll_fn::PollFn<<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}::{closure#0}> as core::futur` |
| 447.4 | 447.6 | 0.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 138.7 | 138.5 | -0.2 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 77.5 | 77.6 | 0.2 | `<tokio::time::interval::Interval>::poll_tick` |
| 208.4 | 208.2 | -0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 111.9 | 111.7 | -0.1 | `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv` |
| 92.9 | 92.8 | -0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 289.8 | 289.9 | 0.1 | `<core::future::poll_fn::PollFn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}:` |
| 35.1 | 35.2 | 0.1 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 513.6 | 513.6 | 0.1 | `__aarch64_ldadd8_relax` |
| 281.8 | 281.8 | -0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 25.3 | 25.2 | -0.0 | `<tokio::sync::notify::Notified as core::future::future::Future>::poll` |
| 336.2 | 336.2 | -0.0 | `<tokio::sync::mpsc::chan::Rx<bacnet_transport::port::ReceivedNpdu, tokio::sync::mpsc::bounded::Semaphore>>::recv` |
| 114.7 | 114.7 | -0.0 | `<tokio::runtime::time::wheel::Wheel>::poll` |
| 0.6 | 0.6 | -0.0 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::lift::LiftFault>` |
| 268.3 | 268.3 | -0.0 | `<tokio::runtime::scheduler::current_thread::Context>::run_task` |
| 199.3 | 199.3 | -0.0 | `<tokio::runtime::task::state::State>::transition_to_idle` |
| 240.3 | 240.3 | -0.0 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule::{closure#0}` |
| 891.5 | 891.4 | -0.0 | `<std::sys::pal::unix::time::Timespec>::now` |
| 190.2 | 190.2 | -0.0 | `<tokio::runtime::task::state::State>::transition_to_running` |
| 55.3 | 55.3 | -0.0 | `<tokio::runtime::time::handle::Handle>::reregister` |
| 192.2 | 192.2 | -0.0 | `<std::thread::local::LocalKey<tokio::runtime::context::Context>>::with::<tokio::runtime::context::thread_rng_n::{closure#0}, u32>` |
| 59.8 | 59.9 | 0.0 | `__aarch64_cas8_acq` |
| 107.8 | 107.9 | 0.0 | `tokio::runtime::task::waker::drop_waker` |
| 176.3 | 176.3 | -0.0 | `<tokio::runtime::task::state::State>::transition_to_notified_by_val` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 1,024.0 | 1,024.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 841.4 | 841.4 | -0.1 | `library/core/src/ptr/mod.rs:847` |
| 746.3 | 746.4 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 523.4 | 523.4 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 477.3 | 477.2 | -0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 427.1 | 427.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 386.9 | 386.9 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 355.0 | 355.0 | 0.0 | `library/core/src/result.rs:2177` |
| 342.2 | 342.2 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 324.0 | 324.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 312.0 | 312.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 307.5 | 307.5 | -0.0 | `library/core/src/cell.rs:558` |
| 303.2 | 303.2 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 283.1 | 283.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 273.6 | 273.6 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 261.0 | 261.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 253.1 | 253.1 | 0.0 | `library/core/src/option.rs:743` |
| 247.0 | 247.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 222.0 | 222.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 207.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 189.2 | 189.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 186.6 | 186.6 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 182.0 | 182.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:2120` |
| 176.1 | 176.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 165.3 | 165.3 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 164.3 | 164.3 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 158.0 | 158.0 | 0.0 | `library/core/src/result.rs:0` |
| 158.0 | 158.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 157.0 | 157.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 156.0 | 156.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 151.0 | 151.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 149.5 | 149.5 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 146.0 | 146.0 | 0.0 | `???:0` |
| 144.1 | 144.1 | -0.0 | `library/core/src/cell.rs:2437` |
| 139.8 | 139.8 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 138.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.9 | 130.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 130.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 130.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 129.0 | 129.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 125.0 | 125.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 121.0 | 121.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 119.5 | 119.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 117.0 | 117.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:1010` |
| 116.0 | 116.0 | 0.0 | `library/core/src/slice/iter/macros.rs:379` |
| 115.0 | 115.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 108.0 | 108.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 107.1 | 107.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 104.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
