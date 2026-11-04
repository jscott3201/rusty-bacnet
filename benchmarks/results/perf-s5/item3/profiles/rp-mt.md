# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,529.1 | 1,399.1 | -130.0 | 22.00 | `_int_free` |
| 1,026.0 | 918.1 | -108.0 | 19.00 | `malloc` |
| 1,002.1 | 918.1 | -84.0 | 22.00 | `free` |
| 865.5 | 865.4 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 848.0 | 831.0 | -17.0 | 46.00 | `__GI_memcpy` |
| 842.6 | 842.7 | 0.1 | 11.13 | `<std::sys::pal::unix::time::Timespec>::now` |
| 697.9 | 697.5 | -0.4 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 555.0 | 503.0 | -52.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 473.1 | 473.0 | -0.1 | 91.00 | `__aarch64_cas4_acq` |
| 468.5 | 468.2 | -0.3 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 451.0 | -13.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 457.0 | 436.9 | -20.1 | 91.40 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 422.4 | 423.2 | 0.7 | 1.36 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 413.0 | 413.0 | -0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.6 | 398.5 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 379.0 | 378.7 | -0.3 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 377.2 | 377.4 | 0.2 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 357.0 | 356.8 | -0.2 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.6 | 346.5 | -0.1 | 68.91 | `__aarch64_swp4_rel` |
| 338.0 | 338.0 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.0 | 319.0 | -0.1 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 281.3 | 281.3 | -0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 280.4 | 280.4 | -0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.2 | 274.3 | 0.1 | 2.00 | `_int_malloc` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.9 | 251.0 | 0.0 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 230.3 | 230.2 | -0.1 | 46.06 | `__aarch64_cas8_acq_rel` |
| 230.0 | 213.0 | -17.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 213.8 | 213.8 | -0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | -0.0 | 42.68 | `__aarch64_ldadd8_rel` |
| 211.9 | 211.5 | -0.4 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.2 | 204.1 | -0.1 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 195.6 | 195.6 | -0.0 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.1 | 193.1 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 181.0 | 181.0 | -0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 177.0 | 177.0 | -0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 164.1 | 163.9 | -0.2 | 11.72 | `tokio::runtime::task::waker::clone_waker` |
| 162.4 | 162.4 | -0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,529.1 | 1,399.1 | -130.0 | `_int_free` |
| 1,026.0 | 918.1 | -108.0 | `malloc` |
| 1,002.1 | 918.1 | -84.0 | `free` |
| 555.0 | 503.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 108.0 | 62.0 | -46.0 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 0.0 | 34.0 | 34.0 | `bacnet_device_core::read::property::encode_ack` |
| 26.0 | 0.0 | -26.0 | `<bacnet_services::read_property::ReadPropertyACK>::encode` |
| 50.0 | 25.0 | -25.0 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 457.0 | 436.9 | -20.1 | `__aarch64_ldadd8_relax` |
| 38.0 | 19.0 | -19.0 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 230.0 | 213.0 | -17.0 | `bacnet_services::read_property::encode_reference` |
| 848.0 | 831.0 | -17.0 | `__GI_memcpy` |
| 14.0 | 0.0 | -14.0 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 464.0 | 451.0 | -13.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 95.0 | 85.0 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 71.0 | 66.0 | -5.0 | `bacnet_encoding::primitives::encode_property_value` |
| 130.5 | 128.7 | -1.8 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 422.4 | 423.2 | 0.7 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 211.9 | 211.5 | -0.4 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 697.9 | 697.5 | -0.4 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 48.6 | 49.0 | 0.4 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 379.0 | 378.7 | -0.3 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 468.5 | 468.2 | -0.3 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 377.2 | 377.4 | 0.2 | `<tokio::runtime::time::Driver>::park_internal` |
| 164.1 | 163.9 | -0.2 | `tokio::runtime::task::waker::clone_waker` |
| 108.0 | 107.8 | -0.2 | `tokio::runtime::task::waker::drop_waker` |
| 357.0 | 356.8 | -0.2 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 82.9 | 82.7 | -0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 93.1 | 93.0 | -0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 137.0 | 136.9 | -0.1 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 850.0 | 836.0 | -14.0 | `library/core/src/ptr/mod.rs:573` |
| 695.6 | 679.5 | -16.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 588.3 | 586.2 | -2.0 | `library/core/src/ptr/mod.rs:847` |
| 536.3 | 536.1 | -0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 383.7 | 383.6 | -0.1 | `library/core/src/sync/atomic.rs:3994` |
| 373.2 | 351.1 | -22.0 | `library/core/src/sync/atomic.rs:4029` |
| 291.6 | 291.6 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.4 | 291.4 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 264.0 | 240.0 | -24.0 | `./malloc/./malloc/malloc.c:4518` |
| 259.3 | 259.2 | -0.1 | `library/std/src/sys/pal/unix/time.rs:107` |
| 255.6 | 255.6 | -0.1 | `library/std/src/panicking.rs:465` |
| 218.8 | 218.7 | -0.1 | `library/core/src/cell.rs:558` |
| 218.5 | 218.5 | -0.0 | `library/core/src/mem/mod.rs:976` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 173.9 | 169.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 171.0 | 153.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 158.3 | 158.2 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 158.2 | 158.2 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 157.4 | 157.5 | 0.1 | `library/core/src/ptr/non_null.rs:454` |
| 156.6 | 156.6 | 0.0 | `???:0` |
| 153.6 | 153.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 152.4 | 152.4 | -0.0 | `library/core/src/option.rs:743` |
| 143.0 | 143.0 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 141.6 | 141.5 | -0.0 | `library/core/src/cell.rs:2437` |
| 136.0 | 126.0 | -10.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 134.1 | 134.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 132.0 | 120.0 | -12.0 | `./malloc/./malloc/malloc.c:4495` |
| 132.0 | 120.0 | -12.0 | `./malloc/./malloc/malloc.c:4527` |
| 130.8 | 130.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 115.0 | 112.0 | -3.0 | `library/core/src/result.rs:2177` |
| 114.0 | 102.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 102.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 102.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 100.0 | -10.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 100.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 103.0 | 95.0 | -8.0 | `./malloc/./malloc/malloc.c:1336` |
| 99.9 | 99.8 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.2 | 99.2 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 98.0 | 98.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 85.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.1 | 94.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 80.0 | -8.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 85.0 | 75.0 | -10.0 | `./malloc/./malloc/arena.c:147` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.1 | 81.1 | -0.0 | `library/core/src/option.rs:2439` |
| 81.0 | 73.0 | -8.0 | `./malloc/./malloc/malloc.c:3163` |
| 80.0 | 72.0 | -8.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 79.6 | 79.6 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 79.0 | 79.0 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 77.6 | 77.5 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
