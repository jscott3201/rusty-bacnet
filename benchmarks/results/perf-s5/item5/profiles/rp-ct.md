# rp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,408.0 | 1,278.0 | -130.0 | 20.00 | `_int_free` |
| 927.4 | 927.0 | -0.4 | 14.64 | `<std::sys::pal::unix::time::Timespec>::now` |
| 918.0 | 810.0 | -108.0 | 17.00 | `malloc` |
| 889.0 | 821.0 | -68.0 | 20.00 | `free` |
| 865.2 | 865.3 | 0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 831.0 | 802.0 | -29.0 | 45.00 | `__GI_memcpy` |
| 697.6 | 697.5 | -0.1 | 8.20 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 532.0 | -2.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 508.0 | 456.0 | -52.0 | 18.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 503.1 | 500.2 | -2.9 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 467.8 | 468.3 | 0.5 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 451.0 | 451.0 | 0.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 436.9 | 417.0 | -19.9 | 87.38 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 545.0 | 132.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.5 | 398.6 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 384.2 | 384.2 | 0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 377.2 | 377.5 | 0.3 | 9.42 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 337.9 | 337.9 | 0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.7 | 328.8 | 0.0 | 65.75 | `__aarch64_cas4_acq` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 298.7 | 298.8 | 0.0 | 59.75 | `__aarch64_swp4_rel` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 281.2 | 281.2 | 0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 278.0 | 278.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 255.2 | 255.2 | 0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 251.0 | 251.0 | -0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | 0.0 | 42.68 | `__aarch64_ldadd8_rel` |
| 213.0 | 213.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 202.0 | 98.0 | -104.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 195.3 | 195.3 | 0.1 | 39.05 | `__aarch64_cas8_acq_rel` |
| 194.3 | 194.4 | 0.0 | 8.00 | `counts::main` |
| 192.0 | 0.0 | -192.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 180.9 | 185.1 | 4.2 | 1.90 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 163.8 | 164.0 | 0.2 | 11.70 | `tokio::runtime::task::waker::clone_waker` |
| 162.3 | 162.3 | 0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 160.2 | 160.2 | 0.0 | 4.00 | `tokio::runtime::task::waker::wake_by_val` |
| 159.0 | 159.0 | 0.0 | 2.00 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 156.0 | 156.0 | 0.0 | 2.00 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 154.2 | 154.2 | 0.0 | 2.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 192.0 | 0.0 | -192.0 | `bacnet_transport::bvll::encode_bvll` |
| 0.0 | 180.0 | 180.0 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 413.0 | 545.0 | 132.0 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 1,408.0 | 1,278.0 | -130.0 | `_int_free` |
| 918.0 | 810.0 | -108.0 | `malloc` |
| 202.0 | 98.0 | -104.0 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 74.0 | 0.0 | -74.0 | `<bytes::bytes_mut::BytesMut as bacnet_bip_codec::sink::BvllSink>::write` |
| 889.0 | 821.0 | -68.0 | `free` |
| 508.0 | 456.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 46.0 | 0.0 | -46.0 | `sendto` |
| 0.0 | 38.0 | 38.0 | `sendmsg` |
| 831.0 | 802.0 | -29.0 | `__GI_memcpy` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<<bacnet_transport::bip::node::NodeHandle>::send_unicast::{closure#0}>` |
| 436.9 | 417.0 | -19.9 | `__aarch64_ldadd8_relax` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu_with_data_attributes::{closure#0}>` |
| 11.0 | 0.0 | -11.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}>` |
| 85.0 | 75.0 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 180.9 | 185.1 | 4.2 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 503.1 | 500.2 | -2.9 | `<tokio::runtime::time::Driver>::park_internal` |
| 534.0 | 532.0 | -2.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 76.0 | 78.0 | 2.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}` |
| 57.0 | 56.0 | -1.0 | `<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu::{closure#0}` |
| 467.8 | 468.3 | 0.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 927.4 | 927.0 | -0.4 | `<std::sys::pal::unix::time::Timespec>::now` |
| 117.5 | 117.9 | 0.4 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.7 | 88.1 | 0.3 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 82.6 | 82.9 | 0.3 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 110.4 | 110.1 | -0.3 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 100.9 | 101.2 | 0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 377.2 | 377.5 | 0.3 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 836.0 | 802.0 | -34.0 | `library/core/src/ptr/mod.rs:573` |
| 651.6 | 635.7 | -15.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 602.9 | 595.9 | -7.0 | `library/core/src/ptr/mod.rs:847` |
| 471.3 | 471.4 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 359.8 | 357.8 | -2.0 | `library/core/src/sync/atomic.rs:3994` |
| 348.1 | 326.2 | -22.0 | `library/core/src/sync/atomic.rs:4029` |
| 267.0 | 267.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.7 | 265.8 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 255.1 | 255.1 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 240.0 | 216.0 | -24.0 | `./malloc/./malloc/malloc.c:4518` |
| 238.0 | 238.0 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 233.4 | 233.5 | 0.1 | `library/core/src/cell.rs:558` |
| 231.0 | 231.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 162.9 | 158.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 158.2 | 158.2 | 0.1 | `library/core/src/sync/atomic.rs:4104` |
| 155.1 | 151.1 | -4.0 | `library/core/src/option.rs:743` |
| 153.0 | 135.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 141.4 | 141.4 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 140.2 | 140.3 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 134.9 | 134.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 118.0 | -8.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 123.8 | 123.9 | 0.0 | `library/core/src/cell.rs:2437` |
| 120.0 | 108.0 | -12.0 | `./malloc/./malloc/malloc.c:4495` |
| 120.0 | 108.0 | -12.0 | `./malloc/./malloc/malloc.c:4527` |
| 117.8 | 117.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 112.5 | 112.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 112.0 | 100.0 | -12.0 | `library/core/src/result.rs:2177` |
| 102.0 | 96.0 | -6.0 | `library/core/src/ptr/non_null.rs:454` |
| 102.0 | 90.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 102.0 | 90.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 102.0 | 90.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:4547` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:4658` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 100.0 | 90.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 99.7 | 99.8 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 87.0 | -8.0 | `./malloc/./malloc/malloc.c:1336` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 91.0 | 91.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 85.0 | 75.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 84.0 | 76.0 | -8.0 | `./malloc/./malloc/arena.c:153` |
| 82.0 | 82.0 | 0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 82.0 | 82.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 81.0 | 81.0 | 0.0 | `library/core/src/option.rs:2439` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:4510` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:4505` |
| 79.5 | 79.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 78.0 | 78.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 77.5 | 77.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 76.0 | 76.0 | 0.0 | `library/core/src/option.rs:0` |
| 74.5 | 74.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
