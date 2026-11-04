# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,529.1 | 1,529.1 | -0.0 | 22.00 | `_int_free` |
| 1,026.0 | 1,026.0 | 0.0 | 19.00 | `malloc` |
| 1,002.1 | 1,002.1 | 0.0 | 22.00 | `free` |
| 914.5 | 841.7 | -72.8 | 13.13 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.3 | 865.4 | 0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 848.0 | 848.0 | 0.0 | 46.00 | `__GI_memcpy` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 634.4 | 697.5 | 63.1 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 555.0 | 555.0 | 0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 473.0 | 473.0 | -0.0 | 90.98 | `__aarch64_cas4_acq` |
| 467.9 | 467.5 | -0.4 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 464.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 456.9 | 456.8 | -0.1 | 91.38 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 422.4 | 422.3 | -0.1 | 1.35 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.5 | 398.4 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 378.3 | 378.1 | -0.2 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 377.1 | 376.9 | -0.2 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 356.9 | 356.8 | -0.0 | 11.15 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.4 | 346.4 | -0.0 | 68.89 | `__aarch64_swp4_rel` |
| 338.0 | 338.0 | 0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.0 | 319.0 | -0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 318.0 | 318.0 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 281.2 | 281.3 | 0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 280.3 | 280.3 | 0.0 | 5.00 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.2 | 274.1 | -0.0 | 2.00 | `_int_malloc` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.9 | 250.9 | -0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 230.2 | 230.1 | -0.0 | 46.03 | `__aarch64_cas8_acq_rel` |
| 230.0 | 230.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | -0.0 | 42.68 | `__aarch64_ldadd8_rel` |
| 211.4 | 211.6 | 0.2 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.2 | 204.1 | -0.1 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 196.7 | 194.8 | -1.8 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.1 | 193.1 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 181.0 | 181.0 | -0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 177.0 | 177.0 | -0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 172.5 | 152.5 | -20.0 | 17.25 | `clock_gettime@@GLIBC_2.17` |
| 170.2 | 0.0 | -170.2 | 4.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 170.2 | 0.0 | -170.2 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 914.5 | 841.7 | -72.8 | `<std::sys::pal::unix::time::Timespec>::now` |
| 634.4 | 697.5 | 63.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 58.1 | 0.0 | -58.1 | `core::ptr::drop_glue::<alloc::vec::into_iter::IntoIter<bacnet_server::server::segmented_receive::ExpiredRequest>>` |
| 172.5 | 152.5 | -20.0 | `clock_gettime@@GLIBC_2.17` |
| 18.0 | 0.0 | -18.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}>` |
| 50.9 | 47.7 | -3.3 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 196.7 | 194.8 | -1.8 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 467.9 | 467.5 | -0.4 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 93.1 | 92.7 | -0.4 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 82.7 | 82.5 | -0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 377.1 | 376.9 | -0.2 | `<tokio::runtime::time::Driver>::park_internal` |
| 211.4 | 211.6 | 0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 107.9 | 107.7 | -0.2 | `tokio::runtime::task::waker::drop_waker` |
| 163.9 | 163.8 | -0.2 | `tokio::runtime::task::waker::clone_waker` |
| 100.9 | 100.8 | -0.2 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 378.3 | 378.1 | -0.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 117.3 | 117.5 | 0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.6 | 87.7 | 0.1 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 398.5 | 398.4 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 0.5 | 0.6 | 0.1 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 137.0 | 137.1 | 0.1 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 129.9 | 130.0 | 0.1 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 0.1 | 0.2 | 0.1 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |
| -0.0 | 0.1 | 0.1 | `malloc_consolidate` |
| 422.4 | 422.3 | -0.1 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 96.7 | 96.8 | 0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 113.3 | 113.2 | -0.1 | `__aarch64_ldadd8_acq_rel` |
| 76.6 | 76.5 | -0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 456.9 | 456.8 | -0.1 | `__aarch64_ldadd8_relax` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 849.0 | 850.0 | 1.0 | `library/core/src/ptr/mod.rs:573` |
| 695.4 | 695.3 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 626.2 | 588.2 | -38.0 | `library/core/src/ptr/mod.rs:847` |
| 536.0 | 536.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 383.5 | 383.6 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 373.1 | 373.1 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 293.3 | 259.3 | -34.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 291.5 | 291.5 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.3 | 291.3 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 264.0 | 264.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 255.5 | 255.5 | -0.0 | `library/std/src/panicking.rs:465` |
| 220.4 | 218.4 | -2.0 | `library/core/src/mem/mod.rs:976` |
| 218.7 | 218.6 | -0.0 | `library/core/src/cell.rs:558` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 173.9 | 173.8 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 171.0 | 171.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 158.3 | 158.1 | -0.1 | `library/core/src/sync/atomic.rs:3995` |
| 158.2 | 158.2 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 157.3 | 157.4 | 0.1 | `library/core/src/ptr/non_null.rs:454` |
| 156.6 | 156.6 | -0.0 | `???:0` |
| 153.6 | 153.6 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 152.4 | 152.4 | -0.0 | `library/core/src/option.rs:743` |
| 142.9 | 142.9 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 141.5 | 141.4 | -0.1 | `library/core/src/cell.rs:2437` |
| 136.0 | 136.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 134.0 | 134.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 132.0 | 132.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 130.7 | 130.7 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 115.0 | 115.0 | 0.0 | `library/core/src/result.rs:2177` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 103.0 | 103.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 99.7 | 99.6 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.1 | 99.1 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 95.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.1 | 94.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 88.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 85.0 | 85.0 | 0.0 | `./malloc/./malloc/arena.c:147` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.0 | 81.0 | 0.0 | `library/core/src/option.rs:2439` |
| 81.0 | 81.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 80.0 | 80.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 79.5 | 79.5 | -0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 79.0 | 79.0 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 77.5 | 77.5 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
