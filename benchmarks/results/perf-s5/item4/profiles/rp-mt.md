# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,269.2 | 1,269.2 | -0.0 | 18.00 | `_int_free` |
| 917.7 | 917.6 | -0.1 | 13.17 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.7 | 865.5 | -0.2 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 846.2 | 846.2 | -0.0 | 18.00 | `free` |
| 810.1 | 810.1 | -0.0 | 15.00 | `malloc` |
| 802.0 | 802.0 | 0.0 | 43.00 | `__GI_memcpy` |
| 686.8 | 686.6 | -0.1 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.5 | 635.2 | -0.3 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 532.0 | 532.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 473.6 | 473.5 | -0.2 | 91.11 | `__aarch64_cas4_acq` |
| 468.2 | 468.1 | -0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 451.0 | 451.0 | -0.0 | 16.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 451.0 | 450.0 | -1.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 428.9 | 427.0 | -1.9 | 1.30 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.1 | 417.0 | -0.0 | 83.42 | `__aarch64_ldadd8_relax` |
| 409.8 | 409.8 | -0.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 381.7 | 380.8 | -0.9 | 4.11 | `<tokio::runtime::time::Driver>::park_internal` |
| 374.1 | 374.0 | -0.2 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.1 | 364.2 | 0.2 | 7.74 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 358.0 | 357.7 | -0.2 | 11.19 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.9 | 346.9 | -0.1 | 68.99 | `__aarch64_swp4_rel` |
| 338.4 | 338.3 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.4 | 319.3 | -0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 291.0 | 301.0 | 10.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.0 | 286.0 | -0.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 0.0 | -284.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 280.9 | 280.7 | -0.2 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 277.0 | 277.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.4 | 274.3 | -0.0 | 2.00 | `_int_malloc` |
| 267.9 | 267.9 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.6 | 250.5 | -0.1 | 50.12 | `__aarch64_cas8_acq_rel` |
| 243.6 | 243.5 | -0.1 | 6.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 235.8 | 235.8 | -0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 234.0 | 234.0 | -0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | -0.0 | 46.69 | `__aarch64_ldadd8_rel` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 216.0 | 216.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 213.0 | 213.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 212.3 | 212.2 | -0.0 | 2.08 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 210.0 | 210.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.3 | 204.3 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 198.0 | 198.1 | 0.1 | 4.03 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.3 | 193.2 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 189.0 | 202.0 | 13.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 180.0 | 180.0 | 0.0 | 2.00 | `<bacnet_transport::bip::datagram::Datagram>::try_send` |
| 177.1 | 177.1 | -0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.1 | 173.0 | -0.1 | 17.31 | `clock_gettime@@GLIBC_2.17` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 284.0 | 0.0 | -284.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 210.0 | 210.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_re` |
| 0.0 | 135.0 | 135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 135.0 | 0.0 | -135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 0.0 | 60.0 | 60.0 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 60.0 | 0.0 | -60.0 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 57.0 | 0.0 | -57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_` |
| 0.0 | 57.0 | 57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_t` |
| 44.0 | 0.0 | -44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::ser` |
| 0.0 | 44.0 | 44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::b` |
| 41.0 | 0.0 | -41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server` |
| 0.0 | 41.0 | 41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpo` |
| 33.0 | 0.0 | -33.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 28.0 | 28.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_re` |
| 0.0 | 22.0 | 22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bac` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<ba` |
| 14.0 | 0.0 | -14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_serv` |
| 0.0 | 14.0 | 14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_trans` |
| 189.0 | 202.0 | 13.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 13.0 | 0.0 | -13.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BACnetServer` |
| 0.0 | 11.0 | 11.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>` |
| 291.0 | 301.0 | 10.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 140.0 | 145.0 | 5.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::dispatch::{closure#0}` |
| 76.0 | 72.0 | -4.0 | `core::ptr::drop_glue::<bacnet_server::server::request_admission::Guard>` |
| 428.9 | 427.0 | -1.9 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 123.3 | 125.1 | 1.8 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 451.0 | 450.0 | -1.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 64.0 | 63.0 | -1.0 | `<bacnet_encoding::apdu::ConfirmedRequest as core::clone::Clone>::clone` |
| 381.7 | 380.8 | -0.9 | `<tokio::runtime::time::Driver>::park_internal` |
| 47.5 | 47.9 | 0.4 | `<tokio::runtime::time::wheel::Wheel>::remove` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 812.1 | 812.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 679.9 | 679.8 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 666.7 | 663.6 | -3.1 | `library/core/src/ptr/mod.rs:847` |
| 552.8 | 552.7 | -0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 381.1 | 381.0 | -0.1 | `library/core/src/sync/atomic.rs:3994` |
| 322.2 | 322.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 294.8 | 294.7 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 294.3 | 294.1 | -0.2 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.0 | 291.9 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 255.9 | 255.9 | -0.1 | `library/std/src/panicking.rs:465` |
| 237.4 | 237.3 | -0.1 | `library/core/src/cell.rs:558` |
| 224.8 | 224.7 | -0.1 | `library/core/src/mem/mod.rs:976` |
| 216.0 | 216.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.4 | 174.4 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 170.0 | 170.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 162.5 | 162.5 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 156.8 | 156.8 | -0.1 | `library/core/src/sync/atomic.rs:4013` |
| 156.7 | 156.6 | -0.0 | `???:0` |
| 156.6 | 156.5 | -0.1 | `library/core/src/option.rs:743` |
| 155.1 | 155.0 | -0.1 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 151.6 | 151.6 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 148.8 | 148.8 | -0.0 | `library/core/src/cell.rs:2437` |
| 138.2 | 138.2 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 135.0 | 135.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 130.9 | 130.9 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 120.0 | 120.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 108.0 | 108.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 108.0 | 108.0 | -0.0 | `./malloc/./malloc/malloc.c:4527` |
| 101.0 | 92.0 | -9.0 | `library/core/src/result.rs:0` |
| 99.8 | 99.8 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.3 | 99.3 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 95.0 | 0.0 | `library/core/src/result.rs:2177` |
| 94.2 | 94.2 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 90.0 | 90.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.1 | 81.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 80.4 | 80.5 | 0.1 | `library/core/src/option.rs:2439` |
| 79.7 | 79.6 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 79.6 | 79.5 | -0.1 | `library/core/src/ptr/mod.rs:1963` |
| 78.0 | 78.0 | -0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 77.6 | 77.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 77.1 | 77.1 | -0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 77.1 | 77.1 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:334` |
| 75.2 | 75.2 | -0.0 | `library/core/src/option.rs:0` |
| 75.0 | 75.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 74.6 | 74.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
