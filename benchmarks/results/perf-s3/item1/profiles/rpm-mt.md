# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,307.3 | 8,293.0 | -14.3 | 125.11 | `_int_free` |
| 6,107.4 | 6,107.3 | -0.0 | 113.00 | `malloc` |
| 4,881.2 | 4,870.6 | -10.5 | 116.00 | `free` |
| 3,773.4 | 3,882.9 | 109.5 | 160.76 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | -0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,628.1 | 2,544.5 | -83.6 | 13.15 | `_int_malloc` |
| 2,122.0 | 2,122.0 | -0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.1 | 2,024.0 | -0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,956.0 | 1,956.7 | 0.7 | 32.84 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.2 | 1,647.2 | -0.1 | 329.45 | `__aarch64_ldadd8_relax` |
| 1,621.4 | 1,621.2 | -0.3 | 40.05 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.8 | 1,540.7 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.6 | 1,234.7 | 0.1 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 947.0 | 0.0 | -947.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 921.3 | 920.0 | -1.2 | 13.21 | `_int_realloc` |
| 918.7 | 917.7 | -1.0 | 13.18 | `<std::sys::pal::unix::time::Timespec>::now` |
| 866.2 | 865.9 | -0.3 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 648.1 | 663.9 | 15.7 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 606.0 | 605.0 | -1.0 | 113.17 | `__aarch64_cas4_acq` |
| 604.1 | 702.7 | 98.6 | 10.23 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 569.9 | 571.0 | 1.1 | 113.98 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 479.4 | 478.4 | -1.0 | 91.08 | `__aarch64_swp4_rel` |
| 468.8 | 468.9 | 0.1 | 10.00 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.8 | 441.5 | 0.7 | 2.73 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 425.4 | 426.8 | 1.5 | 1.35 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.9 | 406.5 | -7.4 | 9.63 | `_int_free_merge_chunk` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.2 | 409.9 | -0.3 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.0 | 379.9 | 0.9 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 375.5 | 374.9 | -0.6 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 947.0 | 0.0 | -947.0 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 65.0 | 189.0 | 124.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 154.0 | 41.0 | -113.0 | `<bacnet_server::server::confirmed_request_tracker::ConfirmedRequestTracker>::begin` |
| 3,773.4 | 3,882.9 | 109.5 | `__GI_memcpy` |
| 604.1 | 702.7 | 98.6 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 2,628.1 | 2,544.5 | -83.6 | `_int_malloc` |
| 0.0 | 59.0 | 59.0 | `<bacnet_endpoint_core::coordinator::CanonicalPeer as core::clone::Clone>::clone` |
| 233.0 | 286.0 | 53.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 263.1 | 286.1 | 23.0 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 87.0 | 109.0 | 22.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of` |
| 248.0 | 268.0 | 20.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 87.0 | 103.0 | 16.0 | `<bacnet_client::client::transaction_peer::TransactionPeer>::of_answer_in::{closure#0}` |
| 648.1 | 663.9 | 15.7 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 130.4 | 145.1 | 14.7 | `_mid_memalign.isra.0` |
| 8,307.3 | 8,293.0 | -14.3 | `_int_free` |
| 4,881.2 | 4,870.6 | -10.5 | `free` |
| 413.9 | 406.5 | -7.4 | `_int_free_merge_chunk` |
| 331.5 | 325.3 | -6.2 | `_int_free_create_chunk` |
| 221.2 | 216.8 | -4.4 | `unlink_chunk.isra.0` |
| 11.2 | 14.6 | 3.4 | `_int_memalign` |
| 78.4 | 81.3 | 3.0 | `_int_free_maybe_consolidate.part.0` |
| 49.3 | 47.2 | -2.1 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 128.6 | 126.6 | -2.0 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 425.4 | 426.8 | 1.5 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 163.5 | 162.3 | -1.2 | `__aarch64_swp8_acq` |
| 921.3 | 920.0 | -1.2 | `_int_realloc` |
| 569.9 | 571.0 | 1.1 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 214.9 | 215.9 | 1.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 918.7 | 917.7 | -1.0 | `<std::sys::pal::unix::time::Timespec>::now` |
| 121.0 | 120.0 | -1.0 | `<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>::admit` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,131.1 | 2,268.1 | 137.0 | `library/core/src/ptr/mod.rs:573` |
| 1,704.3 | 1,700.3 | -4.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,664.3 | 1,664.2 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,605.9 | 1,605.8 | -0.2 | `library/core/src/ptr/mod.rs:847` |
| 1,501.2 | 1,500.1 | -1.2 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 750.6 | 750.1 | -0.6 | `./malloc/./malloc/malloc.c:4495` |
| 745.4 | 745.0 | -0.4 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | -0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 648.2 | 647.1 | -1.1 | `./malloc/./malloc/malloc.c:1336` |
| 621.8 | 621.4 | -0.4 | `./malloc/./malloc/malloc.c:4658` |
| 620.1 | 620.0 | -0.2 | `./malloc/./malloc/malloc.c:4547` |
| 580.1 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.1 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.1 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 570.0 | 565.8 | -4.2 | `???:0` |
| 565.0 | 565.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 553.5 | 553.2 | -0.3 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 517.7 | 505.4 | -12.3 | `./malloc/./malloc/arena.c:147` |
| 500.4 | 500.1 | -0.4 | `./malloc/./malloc/malloc.c:4505` |
| 500.4 | 500.1 | -0.4 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | -0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3316` |
| 451.5 | 452.8 | 1.4 | `./malloc/./malloc/malloc.c:3163` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 421.0 | 244.0 | -177.0 | `library/core/src/slice/iter/macros.rs:180` |
| 416.1 | 416.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 401.4 | 404.8 | 3.3 | `./malloc/./malloc/arena.c:153` |
| 396.9 | 396.8 | -0.2 | `library/core/src/num/uint_macros.rs:2652` |
| 389.3 | 342.7 | -46.6 | `library/core/src/ptr/mod.rs:1963` |
| 385.5 | 383.3 | -2.2 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 353.0 | 352.9 | -0.1 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | -0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | -0.0 | `./malloc/./malloc/malloc.c:3398` |
| 341.4 | 342.9 | 1.4 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 295.1 | 294.9 | -0.2 | `library/core/src/sync/atomic.rs:4083` |
| 294.5 | 294.2 | -0.3 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.4 | 292.2 | -0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 287.2 | 279.2 | -8.0 | `library/core/src/ptr/mut_ptr.rs:970` |
