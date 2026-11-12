# rp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,278.0 | 1,278.0 | 0.0 | 18.00 | `_int_free` |
| 865.3 | 878.8 | 13.5 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 821.0 | 821.0 | 0.0 | 18.00 | `free` |
| 810.0 | 810.0 | 0.0 | 15.00 | `malloc` |
| 804.5 | 805.2 | 0.7 | 12.67 | `<std::sys::pal::unix::time::Timespec>::now` |
| 802.0 | 802.0 | 0.0 | 43.00 | `__GI_memcpy` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 697.4 | 697.5 | 0.0 | 8.20 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 497.1 | 502.3 | 5.3 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 468.5 | 296.4 | -172.1 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 456.0 | 456.0 | 0.0 | 16.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 451.0 | 451.0 | 0.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.0 | 399.4 | -17.6 | 83.39 | `__aarch64_ldadd8_relax` |
| 398.7 | 224.0 | -174.7 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 377.3 | 289.8 | -87.4 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 337.8 | 337.9 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.7 | 328.7 | -0.0 | 65.75 | `__aarch64_cas4_acq` |
| 320.2 | 320.2 | 0.0 | 10.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 298.7 | 298.7 | -0.0 | 59.75 | `__aarch64_swp4_rel` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 281.2 | 281.2 | 0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 278.0 | 278.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.2 | 255.2 | 0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 251.0 | 251.0 | 0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 214.0 | 214.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | 0.0 | 42.67 | `__aarch64_ldadd8_rel` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 195.3 | 177.7 | -17.6 | 39.06 | `__aarch64_cas8_acq_rel` |
| 194.3 | 194.3 | -0.0 | 8.00 | `counts::main` |
| 189.6 | 181.8 | -7.8 | 1.99 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 180.0 | 180.0 | 0.0 | 2.00 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 164.1 | 114.8 | -49.3 | 11.72 | `tokio::runtime::task::waker::clone_waker` |
| 162.2 | 162.2 | 0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 160.1 | 160.1 | 0.0 | 4.00 | `tokio::runtime::task::waker::wake_by_val` |
| 159.0 | 159.0 | 0.0 | 2.00 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 156.0 | 156.0 | 0.0 | 2.00 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 154.1 | 154.1 | 0.0 | 2.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 148.0 | 148.0 | 0.0 | 4.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::ops::drop::Drop>::drop` |
| 147.0 | 147.0 | -0.0 | 7.00 | `bcmp` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 398.7 | 224.0 | -174.7 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.5 | 296.4 | -172.1 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.7 | 0.0 | -117.7 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.9 | 0.0 | -87.9 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 377.3 | 289.8 | -87.4 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 76.7 | 0.0 | -76.7 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.0 | 72.0 | 72.0 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.0 | 63.0 | 63.0 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closure#0}::{cl` |
| 108.1 | 58.8 | -49.3 | `tokio::runtime::task::waker::drop_waker` |
| 164.1 | 114.8 | -49.3 | `tokio::runtime::task::waker::clone_waker` |
| 93.3 | 62.0 | -31.3 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 83.6 | 66.0 | -17.6 | `__aarch64_ldadd8_acq_rel` |
| 417.0 | 399.4 | -17.6 | `__aarch64_ldadd8_relax` |
| 59.9 | 42.4 | -17.6 | `__aarch64_cas8_acq` |
| 195.3 | 177.7 | -17.6 | `__aarch64_cas8_acq_rel` |
| 865.3 | 878.8 | 13.5 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 189.6 | 181.8 | -7.8 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 497.1 | 502.3 | 5.3 | `<tokio::runtime::time::Driver>::park_internal` |
| 92.9 | 91.9 | -1.0 | `counts::main::{closure#0}` |
| 804.5 | 805.2 | 0.7 | `<std::sys::pal::unix::time::Timespec>::now` |
| 109.7 | 110.3 | 0.5 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 34.4 | 34.7 | 0.3 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 101.0 | 100.7 | -0.2 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 82.7 | 82.6 | -0.1 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 96.7 | 96.7 | -0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 697.4 | 697.5 | 0.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 51.3 | 51.3 | 0.0 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 251.0 | 251.0 | 0.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 44.0 | 44.0 | 0.0 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 337.8 | 337.9 | 0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 803.0 | 803.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 635.6 | 607.5 | -28.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 595.8 | 597.3 | 1.5 | `library/core/src/ptr/mod.rs:847` |
| 471.4 | 443.3 | -28.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 357.8 | 357.8 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 326.2 | 315.6 | -10.5 | `library/core/src/sync/atomic.rs:4029` |
| 267.0 | 267.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.7 | 265.7 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 238.0 | 215.5 | -22.5 | `library/core/src/mem/mod.rs:976` |
| 233.5 | 204.4 | -29.1 | `library/core/src/cell.rs:558` |
| 231.0 | 231.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 221.1 | 221.1 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 216.0 | 216.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 158.9 | 151.9 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 158.2 | 140.7 | -17.6 | `library/core/src/sync/atomic.rs:4104` |
| 151.1 | 145.8 | -5.2 | `library/core/src/option.rs:743` |
| 141.5 | 136.2 | -5.3 | `library/core/src/sync/atomic.rs:3995` |
| 140.2 | 140.2 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 135.0 | 135.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 134.9 | 121.4 | -13.5 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 123.9 | 112.1 | -11.8 | `library/core/src/cell.rs:2437` |
| 118.0 | 118.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 117.9 | 110.8 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 112.5 | 112.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 108.0 | 108.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 108.0 | 108.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 99.8 | 64.7 | -35.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 97.0 | 97.0 | 0.0 | `library/core/src/result.rs:2177` |
| 96.0 | 98.3 | 2.2 | `library/core/src/ptr/non_null.rs:454` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 91.0 | 91.0 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 90.0 | 90.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 87.0 | 83.5 | -3.5 | `library/core/src/mem/mod.rs:975` |
| 82.0 | 82.0 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 81.0 | 81.0 | 0.0 | `library/core/src/option.rs:2439` |
| 79.5 | 73.0 | -6.5 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 78.0 | 69.0 | -9.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 77.6 | 54.0 | -23.6 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 76.0 | 76.0 | 0.0 | `library/core/src/option.rs:0` |
| 76.0 | 76.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 75.0 | 75.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 74.6 | 56.0 | -18.6 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
| 74.0 | 74.0 | 0.0 | `crates/bacnet-network/src/layer_receive.rs:89` |
| 73.3 | 50.0 | -23.3 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:686` |
| 72.0 | 72.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 72.0 | 72.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 72.0 | 72.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
