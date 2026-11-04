# rp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,278.0 | 1,278.0 | 0.0 | 18.00 | `_int_free` |
| 997.8 | 997.2 | -0.6 | 16.64 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.3 | 865.3 | -0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 822.0 | 822.0 | 0.0 | 18.00 | `free` |
| 810.0 | 810.0 | 0.0 | 15.00 | `malloc` |
| 802.0 | 802.0 | 0.0 | 43.00 | `__GI_memcpy` |
| 686.3 | 686.3 | -0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 634.4 | 634.4 | 0.1 | 9.20 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 532.0 | 532.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 504.4 | 500.3 | -4.1 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 467.6 | 467.9 | 0.2 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 456.0 | 456.0 | 0.0 | 16.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 451.0 | 450.0 | -1.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 416.9 | 417.0 | 0.1 | 83.38 | `__aarch64_ldadd8_relax` |
| 409.3 | 409.2 | -0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 384.3 | 384.2 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 372.4 | 372.9 | 0.5 | 9.42 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.4 | 364.3 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 337.9 | 337.9 | -0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.8 | 328.8 | -0.0 | 65.76 | `__aarch64_cas4_acq` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 298.8 | 298.8 | -0.0 | 59.76 | `__aarch64_swp4_rel` |
| 291.0 | 301.0 | 10.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.0 | 286.0 | 0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.6 | 284.7 | 0.1 | 7.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 278.0 | 0.0 | -278.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 277.0 | 277.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 268.0 | 268.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 255.2 | 255.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 235.8 | 235.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | 0.0 | 46.67 | `__aarch64_ldadd8_rel` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 216.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 215.2 | 215.3 | 0.1 | 43.05 | `__aarch64_cas8_acq_rel` |
| 213.0 | 213.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 210.0 | 210.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 194.4 | 194.4 | -0.0 | 8.00 | `counts::main` |
| 189.0 | 202.0 | 13.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 180.0 | 180.0 | 0.0 | 2.00 | `<bacnet_transport::bip::datagram::Datagram>::try_send` |
| 179.3 | 185.4 | 6.1 | 1.88 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 170.3 | 170.3 | -0.0 | 4.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 170.1 | 170.1 | -0.0 | 17.01 | `clock_gettime@@GLIBC_2.17` |
| 163.9 | 163.9 | 0.0 | 11.71 | `tokio::runtime::task::waker::clone_waker` |
| 162.4 | 162.4 | -0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 160.2 | 160.2 | -0.0 | 4.01 | `tokio::runtime::task::waker::wake_by_val` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 278.0 | 0.0 | -278.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 204.0 | 204.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_re` |
| 0.0 | 135.0 | 135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 135.0 | 0.0 | -135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 82.9 | 0.0 | -82.9 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 0.0 | 82.9 | 82.9 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 0.0 | 57.0 | 57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_t` |
| 57.0 | 0.0 | -57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_` |
| 44.0 | 0.0 | -44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::ser` |
| 0.0 | 44.0 | 44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::b` |
| 41.0 | 0.0 | -41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server` |
| 0.0 | 41.0 | 41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpo` |
| 33.0 | 0.0 | -33.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 28.0 | 28.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_re` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<ba` |
| 0.0 | 22.0 | 22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bac` |
| 14.0 | 0.0 | -14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_serv` |
| 0.0 | 14.0 | 14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_trans` |
| 189.0 | 202.0 | 13.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 13.0 | 0.0 | -13.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BACnetServer` |
| 0.0 | 11.0 | 11.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>` |
| 291.0 | 301.0 | 10.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 179.3 | 185.4 | 6.1 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 140.0 | 145.0 | 5.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::dispatch::{closure#0}` |
| 504.4 | 500.3 | -4.1 | `<tokio::runtime::time::Driver>::park_internal` |
| 76.0 | 72.0 | -4.0 | `core::ptr::drop_glue::<bacnet_server::server::request_admission::Guard>` |
| 451.0 | 450.0 | -1.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 64.0 | 63.0 | -1.0 | `<bacnet_encoding::apdu::ConfirmedRequest as core::clone::Clone>::clone` |
| 997.8 | 997.2 | -0.6 | `<std::sys::pal::unix::time::Timespec>::now` |
| 116.9 | 117.5 | 0.6 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 812.0 | 812.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 683.9 | 680.9 | -3.0 | `library/core/src/ptr/mod.rs:847` |
| 651.6 | 651.7 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 487.4 | 487.4 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 356.9 | 356.8 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 319.1 | 319.1 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 289.2 | 289.2 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 268.8 | 268.8 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.1 | 267.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 251.9 | 252.0 | 0.1 | `library/core/src/cell.rs:558` |
| 244.0 | 244.0 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 231.0 | 231.0 | -0.0 | `library/std/src/panicking.rs:465` |
| 216.0 | 216.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.2 | 174.2 | 0.1 | `library/core/src/sync/atomic.rs:4104` |
| 162.9 | 162.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 159.1 | 159.1 | 0.0 | `library/core/src/option.rs:743` |
| 146.9 | 146.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 145.4 | 145.4 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 143.3 | 143.3 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 135.0 | 135.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 130.8 | 130.8 | 0.0 | `library/core/src/cell.rs:2437` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 121.8 | 121.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 120.0 | 120.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 112.5 | 112.5 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 108.0 | 108.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 108.0 | 108.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 101.0 | 92.0 | -9.0 | `library/core/src/result.rs:0` |
| 99.7 | 99.7 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 96.0 | 96.0 | 0.0 | `library/core/src/ptr/non_null.rs:454` |
| 95.0 | 95.0 | 0.0 | `library/core/src/result.rs:2177` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 91.0 | 91.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 90.0 | 90.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 87.0 | 87.0 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 82.1 | 82.0 | -0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 82.0 | 82.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 82.0 | 82.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 81.1 | 81.0 | -0.0 | `library/core/src/option.rs:2439` |
| 81.0 | 81.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 79.6 | 79.5 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 77.4 | 77.5 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 77.0 | 77.0 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 77.0 | 77.0 | -0.0 | `library/core/src/option.rs:0` |
| 76.0 | 76.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 75.0 | 75.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 74.4 | 74.5 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
| 74.0 | 74.0 | 0.0 | `crates/bacnet-network/src/layer_receive.rs:89` |
