# rp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,538.0 | 1,408.0 | -130.0 | 22.00 | `_int_free` |
| 1,026.0 | 918.0 | -108.0 | 19.00 | `malloc` |
| 963.0 | 889.0 | -74.0 | 22.00 | `free` |
| 927.5 | 927.8 | 0.3 | 14.64 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.3 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 848.0 | 831.0 | -17.0 | 46.00 | `__GI_memcpy` |
| 697.7 | 697.6 | -0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 560.0 | 508.0 | -52.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 502.4 | 504.6 | 2.2 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 468.2 | 468.4 | 0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 451.0 | -13.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 457.0 | 437.0 | -20.0 | 91.39 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.4 | 398.6 | 0.2 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 384.3 | 384.3 | 0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 377.5 | 377.8 | 0.2 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 337.9 | 337.9 | -0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.8 | 328.8 | -0.0 | 65.76 | `__aarch64_cas4_acq` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 298.8 | 298.8 | -0.0 | 59.76 | `__aarch64_swp4_rel` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 281.3 | 281.3 | -0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 278.0 | 278.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.3 | 255.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 251.0 | 251.0 | -0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 230.0 | 213.0 | -17.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | -0.0 | 42.68 | `__aarch64_ldadd8_rel` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 195.3 | 195.3 | 0.0 | 39.06 | `__aarch64_cas8_acq_rel` |
| 194.4 | 194.4 | -0.0 | 8.01 | `counts::main` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 182.5 | 179.1 | -3.4 | 1.92 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 164.0 | 164.1 | 0.1 | 11.72 | `tokio::runtime::task::waker::clone_waker` |
| 162.4 | 162.4 | 0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 160.2 | 160.2 | -0.0 | 4.01 | `tokio::runtime::task::waker::wake_by_val` |
| 159.0 | 159.0 | 0.0 | 2.00 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 156.0 | 156.0 | 0.0 | 2.00 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 154.2 | 154.2 | 0.0 | 2.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,538.0 | 1,408.0 | -130.0 | `_int_free` |
| 1,026.0 | 918.0 | -108.0 | `malloc` |
| 963.0 | 889.0 | -74.0 | `free` |
| 560.0 | 508.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 108.0 | 62.0 | -46.0 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 0.0 | 34.0 | 34.0 | `bacnet_device_core::read::property::encode_ack` |
| 26.0 | 0.0 | -26.0 | `<bacnet_services::read_property::ReadPropertyACK>::encode` |
| 50.0 | 25.0 | -25.0 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 457.0 | 437.0 | -20.0 | `__aarch64_ldadd8_relax` |
| 38.0 | 19.0 | -19.0 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 230.0 | 213.0 | -17.0 | `bacnet_services::read_property::encode_reference` |
| 848.0 | 831.0 | -17.0 | `__GI_memcpy` |
| 14.0 | 0.0 | -14.0 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 464.0 | 451.0 | -13.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 95.0 | 85.0 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 71.0 | 66.0 | -5.0 | `bacnet_encoding::primitives::encode_property_value` |
| 182.5 | 179.1 | -3.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 502.4 | 504.6 | 2.2 | `<tokio::runtime::time::Driver>::park_internal` |
| 137.0 | 136.3 | -0.6 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 101.2 | 101.6 | 0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 96.7 | 96.3 | -0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 927.5 | 927.8 | 0.3 | `<std::sys::pal::unix::time::Timespec>::now` |
| 93.1 | 93.3 | 0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 110.3 | 110.5 | 0.2 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 82.9 | 83.1 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 377.5 | 377.8 | 0.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 398.4 | 398.6 | 0.2 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.2 | 468.4 | 0.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 76.4 | 76.6 | 0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 865.4 | 865.3 | -0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 850.0 | 836.0 | -14.0 | `library/core/src/ptr/mod.rs:573` |
| 667.7 | 651.8 | -15.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 604.9 | 602.9 | -2.0 | `library/core/src/ptr/mod.rs:847` |
| 471.5 | 471.5 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 370.2 | 348.2 | -22.0 | `library/core/src/sync/atomic.rs:4029` |
| 359.9 | 359.8 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 267.1 | 267.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.8 | 265.8 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 264.0 | 240.0 | -24.0 | `./malloc/./malloc/malloc.c:4518` |
| 255.1 | 255.1 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 238.0 | 238.1 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 233.5 | 233.5 | 0.0 | `library/core/src/cell.rs:558` |
| 231.0 | 231.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 171.0 | 153.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 166.9 | 162.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 158.2 | 158.2 | 0.0 | `library/core/src/sync/atomic.rs:4104` |
| 155.1 | 155.1 | 0.0 | `library/core/src/option.rs:743` |
| 141.5 | 141.4 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 140.3 | 140.3 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 136.0 | 126.0 | -10.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 134.9 | 134.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 132.0 | 120.0 | -12.0 | `./malloc/./malloc/malloc.c:4527` |
| 132.0 | 120.0 | -12.0 | `./malloc/./malloc/malloc.c:4495` |
| 129.0 | 129.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 123.9 | 123.9 | 0.0 | `library/core/src/cell.rs:2437` |
| 117.9 | 117.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 115.0 | 112.0 | -3.0 | `library/core/src/result.rs:2177` |
| 114.0 | 102.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 102.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 102.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 112.5 | 112.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 100.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 103.0 | 95.0 | -8.0 | `./malloc/./malloc/malloc.c:1336` |
| 102.0 | 102.0 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 99.8 | 99.8 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 85.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 92.0 | 84.0 | -8.0 | `./malloc/./malloc/arena.c:153` |
| 91.0 | 91.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 82.1 | 82.1 | 0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 82.0 | 82.0 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 81.1 | 81.1 | -0.0 | `library/core/src/option.rs:2439` |
| 80.0 | 72.0 | -8.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 79.6 | 79.6 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 78.0 | 70.0 | -8.0 | `./malloc/./malloc/malloc.c:3163` |
| 78.0 | 78.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 77.6 | 77.6 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
