# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,185.3 | 8,180.3 | -5.0 | 125.57 | `_int_free` |
| 6,101.0 | 6,101.0 | -0.0 | 113.00 | `malloc` |
| 4,481.0 | 4,482.1 | 1.0 | 116.00 | `free` |
| 3,896.5 | 3,893.1 | -3.4 | 168.12 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 3,004.0 | 3,004.0 | -0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,659.3 | 2,781.8 | 122.5 | 13.12 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,022.0 | -2.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,738.0 | 1,748.5 | 10.5 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.1 | -0.0 | 329.42 | `__aarch64_ldadd8_relax` |
| 1,620.9 | 1,620.8 | -0.1 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.5 | 1,540.4 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 998.0 | 997.9 | -0.1 | 16.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 908.8 | 899.9 | -9.0 | 13.00 | `_int_realloc` |
| 865.6 | 865.5 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 663.6 | 258.5 | -405.1 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.0 | 635.0 | -0.1 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 565.0 | 565.0 | -0.0 | 113.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 501.9 | 502.3 | 0.5 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.6 | 468.5 | -0.1 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 435.9 | 436.7 | 0.8 | 9.54 | `_int_free_merge_chunk` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.5 | 81.5 | -328.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.5 | 384.5 | -0.0 | 12.02 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 373.5 | 378.2 | 4.7 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 398.6 | 34.4 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 338.7 | 358.1 | 19.4 | 9.54 | `_int_free_create_chunk` |
| 338.2 | 338.2 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 663.6 | 258.5 | -405.1 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 409.5 | 81.5 | -328.0 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 286.0 | 0.0 | -286.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 14.0 | 176.0 | 162.0 | `<bytes::bytes_mut::BytesMut>::with_capacity` |
| 2,659.3 | 2,781.8 | 122.5 | `_int_malloc` |
| 218.0 | 102.0 | -116.0 | `bacnet_server::server::requests::issue::response_limits` |
| 270.7 | 369.2 | 98.6 | `unlink_chunk.isra.0` |
| 60.0 | 156.0 | 96.0 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 28.0 | 107.0 | 79.0 | `<smallvec::SmallVec<[u8; 6]>>::from_slice` |
| 282.0 | 225.0 | -57.0 | `malloc_consolidate` |
| 268.0 | 318.0 | 50.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 37.0 | 86.0 | 49.0 | `bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}` |
| 92.0 | 45.0 | -47.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 93.0 | 134.0 | 41.0 | `<tokio::sync::notify::Notified as core::ops::drop::Drop>::drop` |
| 50.1 | 10.1 | -40.0 | `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop` |
| 17.0 | 53.0 | 36.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}>` |
| 216.0 | 181.0 | -35.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 364.2 | 398.6 | 34.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 284.7 | 251.0 | -33.7 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 120.0 | 90.0 | -30.0 | `__aarch64_swp8_acq` |
| 136.0 | 159.0 | 23.0 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 235.8 | 213.8 | -22.0 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 93.0 | 72.0 | -21.0 | `<bacnet_client::tsm::Tsm>::complete_transaction_inner` |
| 215.5 | 195.5 | -20.0 | `__aarch64_cas8_acq_rel` |
| 233.5 | 213.5 | -20.0 | `__aarch64_ldadd8_rel` |
| 0.0 | 20.0 | 20.0 | `<bacnet_client::sync_mutex::SyncMutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock` |
| 338.7 | 358.1 | 19.4 | `_int_free_create_chunk` |
| 0.0 | 17.0 | 17.0 | `core::ptr::drop_glue::<tokio::util::sharded_list::ShardGuard<tokio::runtime::task::Task<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>>>` |
| 0.0 | 16.0 | 16.0 | `<core::result::Result<(u8, bacnet_client::tsm::TransactionRegistration), bacnet_client::tsm::coordinated::CoordinatedRegistrationError>>::map_err::<bacnet_types` |
| 74.0 | 59.0 | -15.0 | `<bacnet_encoding::apdu::ComplexAck as core::clone::Clone>::clone` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,331.1 | 2,320.1 | -11.0 | `library/core/src/ptr/mod.rs:573` |
| 1,697.2 | 1,704.2 | 7.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,635.9 | 1,619.9 | -16.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,602.2 | 1,552.2 | -50.0 | `library/core/src/ptr/mod.rs:847` |
| 1,506.8 | 1,506.1 | -0.8 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,095.0 | 5.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 888.0 | -2.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 753.4 | 753.0 | -0.4 | `./malloc/./malloc/malloc.c:4495` |
| 748.5 | 748.0 | -0.5 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 734.0 | -2.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 664.0 | -47.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 645.2 | 644.0 | -1.1 | `./malloc/./malloc/malloc.c:1336` |
| 622.9 | 622.5 | -0.4 | `./malloc/./malloc/malloc.c:4547` |
| 608.5 | 608.5 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 580.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 574.0 | 574.0 | -0.0 | `./malloc/./malloc/arena.c:153` |
| 565.0 | 565.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 502.3 | 502.0 | -0.3 | `./malloc/./malloc/malloc.c:4510` |
| 502.3 | 502.0 | -0.3 | `./malloc/./malloc/malloc.c:4505` |
| 487.7 | 471.7 | -16.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | -0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 454.4 | 454.5 | 0.1 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | -0.0 | `library/core/src/cmp.rs:1051` |
| 410.6 | 410.5 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 409.0 | 405.0 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 359.0 | 360.0 | 1.0 | `library/core/src/sync/atomic.rs:3994` |
| 351.7 | 352.0 | 0.3 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | -0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | -0.0 | `./malloc/./malloc/malloc.c:3395` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 343.4 | 343.5 | 0.1 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 289.4 | 289.3 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 285.0 | 277.0 | -8.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 283.0 | 280.0 | -3.0 | `library/core/src/result.rs:2178` |
| 270.0 | 270.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 268.9 | 265.9 | -3.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.2 | 267.2 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 262.0 | 262.0 | -0.0 | `library/core/src/num/uint_macros.rs:3212` |
