# rp / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,538.0 | 1,538.0 | 0.0 | 22.00 | `_int_free` |
| 1,026.0 | 1,026.0 | 0.0 | 19.00 | `malloc` |
| 996.5 | 996.5 | -0.0 | 16.67 | `<std::sys::pal::unix::time::Timespec>::now` |
| 964.0 | 963.0 | -1.0 | 22.00 | `free` |
| 865.2 | 865.3 | 0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 848.0 | 848.0 | 0.0 | 46.00 | `__GI_memcpy` |
| 686.2 | 281.2 | -405.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 634.5 | 634.3 | -0.2 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 560.0 | 560.0 | 0.0 | 20.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 496.8 | 496.8 | -0.0 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 467.6 | 467.7 | 0.1 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 464.0 | 464.0 | 0.0 | 7.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 456.9 | 456.9 | 0.0 | 91.37 | `__aarch64_ldadd8_relax` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 409.2 | 81.2 | -328.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 384.2 | 384.2 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 372.3 | 377.5 | 5.1 | 9.42 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.0 | 398.4 | 34.4 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 338.0 | 338.0 | 0.0 | 2.00 | `_int_malloc` |
| 337.9 | 337.9 | -0.0 | 4.00 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.7 | 328.7 | 0.0 | 65.75 | `__aarch64_cas4_acq` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 298.7 | 298.7 | 0.0 | 59.75 | `__aarch64_swp4_rel` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 286.0 | 0.0 | -286.0 | 4.00 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.7 | 251.0 | -33.7 | 7.34 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 278.0 | 278.0 | 0.0 | 2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 277.0 | 279.0 | 2.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 268.0 | 318.0 | 50.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 255.2 | 255.2 | -0.0 | 5.00 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 235.8 | 213.8 | -22.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 213.4 | -20.0 | 46.67 | `__aarch64_ldadd8_rel` |
| 230.0 | 230.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 219.3 | 219.3 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>` |
| 216.0 | 181.0 | -35.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 215.2 | 195.2 | -20.0 | 43.05 | `__aarch64_cas8_acq_rel` |
| 210.0 | 211.0 | 1.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 202.0 | 202.0 | 0.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 194.3 | 194.3 | -0.0 | 8.00 | `counts::main` |
| 192.0 | 192.0 | 0.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.9 | 189.9 | 0.0 | 2.00 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 170.2 | 170.2 | -0.0 | 4.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 170.1 | 170.1 | -0.0 | 17.01 | `clock_gettime@@GLIBC_2.17` |
| 163.8 | 164.0 | 0.1 | 11.70 | `tokio::runtime::task::waker::clone_waker` |
| 162.2 | 162.2 | -0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 686.2 | 281.2 | -405.0 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 409.2 | 81.2 | -328.0 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 286.0 | 0.0 | -286.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 60.0 | 156.0 | 96.0 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 28.0 | 107.0 | 79.0 | `<smallvec::SmallVec<[u8; 6]>>::from_slice` |
| 268.0 | 318.0 | 50.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 37.0 | 86.0 | 49.0 | `bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}` |
| 92.0 | 45.0 | -47.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 93.0 | 134.0 | 41.0 | `<tokio::sync::notify::Notified as core::ops::drop::Drop>::drop` |
| 50.0 | 10.0 | -40.0 | `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop` |
| 17.0 | 53.0 | 36.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}>` |
| 216.0 | 181.0 | -35.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 364.0 | 398.4 | 34.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 284.7 | 251.0 | -33.7 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 0.0 | 32.0 | 32.0 | `<bytes::bytes_mut::BytesMut>::with_capacity` |
| 136.0 | 159.0 | 23.0 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 235.8 | 213.8 | -22.0 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 93.0 | 72.0 | -21.0 | `<bacnet_client::tsm::Tsm>::complete_transaction_inner` |
| 215.2 | 195.2 | -20.0 | `__aarch64_cas8_acq_rel` |
| 0.0 | 20.0 | 20.0 | `<bacnet_client::sync_mutex::SyncMutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock` |
| 233.4 | 213.4 | -20.0 | `__aarch64_ldadd8_rel` |
| 0.0 | 17.0 | 17.0 | `core::ptr::drop_glue::<tokio::util::sharded_list::ShardGuard<tokio::runtime::task::Task<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>>>` |
| 0.0 | 16.0 | 16.0 | `<core::result::Result<(u8, bacnet_client::tsm::TransactionRegistration), bacnet_client::tsm::coordinated::CoordinatedRegistrationError>>::map_err::<bacnet_types` |
| 74.0 | 59.0 | -15.0 | `<bacnet_encoding::apdu::ComplexAck as core::clone::Clone>::clone` |
| 12.0 | 0.0 | -12.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}>` |
| 103.0 | 93.0 | -10.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::take_current_reassembly::{closure#0}>` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}>` |
| 9.0 | 0.0 | -9.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::current_reassembly_owner::{closure#0}>` |
| 109.0 | 101.0 | -8.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 860.0 | 849.0 | -11.0 | `library/core/src/ptr/mod.rs:573` |
| 692.8 | 642.8 | -50.0 | `library/core/src/ptr/mod.rs:847` |
| 683.5 | 667.5 | -16.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 487.3 | 471.3 | -16.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 363.1 | 370.1 | 7.0 | `library/core/src/sync/atomic.rs:4029` |
| 358.8 | 359.8 | 1.0 | `library/core/src/sync/atomic.rs:3994` |
| 289.1 | 289.1 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 268.7 | 265.7 | -3.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.0 | 267.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 264.0 | 264.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 251.9 | 233.4 | -18.5 | `library/core/src/cell.rs:558` |
| 244.0 | 240.0 | -4.0 | `library/core/src/mem/mod.rs:976` |
| 231.0 | 231.0 | 0.0 | `library/std/src/panicking.rs:465` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.1 | 158.1 | -16.0 | `library/core/src/sync/atomic.rs:4104` |
| 171.0 | 171.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 170.9 | 166.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 163.0 | 155.1 | -8.0 | `library/core/src/option.rs:743` |
| 146.9 | 134.9 | -12.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 145.4 | 141.4 | -4.0 | `library/core/src/sync/atomic.rs:3995` |
| 143.2 | 140.2 | -3.0 | `library/core/src/sync/atomic.rs:4013` |
| 138.0 | 136.0 | -2.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 132.0 | 132.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 130.8 | 123.8 | -7.0 | `library/core/src/cell.rs:2437` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 121.8 | 117.8 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 114.0 | 114.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 112.5 | 112.5 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 110.0 | 110.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 110.0 | 110.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 110.0 | 115.0 | 5.0 | `library/core/src/result.rs:2177` |
| 107.0 | 69.0 | -38.0 | `library/core/src/result.rs:0` |
| 103.0 | 103.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 102.0 | 102.0 | -0.0 | `library/core/src/ptr/non_null.rs:454` |
| 99.7 | 99.7 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 95.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 92.0 | 92.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 91.0 | 91.0 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 88.0 | 88.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 82.0 | 82.0 | -0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 82.0 | 82.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 82.0 | 78.0 | -4.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 81.0 | 81.0 | -0.0 | `library/core/src/option.rs:2439` |
| 81.0 | 29.0 | -52.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/batch_semaphore.rs:0` |
| 80.0 | 80.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 79.5 | 79.6 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
