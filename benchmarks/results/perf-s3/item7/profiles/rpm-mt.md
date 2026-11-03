# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,294.5 | 8,300.2 | 5.7 | 125.00 | `_int_free` |
| 6,107.4 | 6,107.3 | -0.1 | 113.00 | `malloc` |
| 4,877.1 | 4,886.7 | 9.6 | 116.00 | `free` |
| 3,885.1 | 3,885.0 | -0.1 | 167.75 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,567.2 | 2,575.0 | 7.8 | 13.07 | `_int_malloc` |
| 2,121.9 | 2,122.0 | 0.1 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.1 | 2,024.1 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,955.4 | 1,962.2 | 6.7 | 32.79 | `realloc` |
| 1,752.0 | 1,752.0 | 0.0 | 4.04 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.1 | 0.1 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | 0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.6 | 1,234.7 | 0.2 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 921.1 | 917.2 | -3.9 | 13.25 | `_int_realloc` |
| 916.6 | 916.9 | 0.3 | 13.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.7 | 865.7 | 0.0 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 702.2 | 635.2 | -67.0 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | -0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 663.7 | 663.6 | -0.0 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 603.4 | 603.4 | -0.0 | 112.74 | `__aarch64_cas4_acq` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.0 | 570.4 | 0.4 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 476.9 | 476.8 | -0.0 | 90.64 | `__aarch64_swp4_rel` |
| 468.1 | 469.0 | 0.9 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 446.2 | 441.8 | -4.3 | 2.71 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 425.6 | 425.9 | 0.3 | 1.34 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.8 | 409.7 | -0.1 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 405.6 | 407.3 | 1.7 | 9.44 | `_int_free_merge_chunk` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 378.9 | 379.0 | 0.1 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 374.7 | 375.0 | 0.3 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.3 | 364.2 | -0.0 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 702.2 | 635.2 | -67.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 42.0 | 28.0 | -14.0 | `core::ptr::drop_glue::<bacnet_network::response_route::ResponseRoute>` |
| 4,877.1 | 4,886.7 | 9.6 | `free` |
| 2,567.2 | 2,575.0 | 7.8 | `_int_malloc` |
| 119.1 | 126.6 | 7.5 | `_mid_memalign.isra.0` |
| 224.2 | 231.4 | 7.2 | `unlink_chunk.isra.0` |
| 1,955.4 | 1,962.2 | 6.7 | `realloc` |
| 318.5 | 324.8 | 6.3 | `_int_free_create_chunk` |
| 8,294.5 | 8,300.2 | 5.7 | `_int_free` |
| 446.2 | 441.8 | -4.3 | `malloc_consolidate` |
| 80.9 | 85.0 | 4.1 | `_int_free_maybe_consolidate.part.0` |
| 921.1 | 917.2 | -3.9 | `_int_realloc` |
| 8.6 | 10.3 | 1.7 | `_int_memalign` |
| 405.6 | 407.3 | 1.7 | `_int_free_merge_chunk` |
| 276.0 | 277.0 | 1.0 | `bacnet_encoding::apdu::decode_apdu` |
| 468.1 | 469.0 | 0.9 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 127.7 | 127.2 | -0.5 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 162.8 | 162.3 | -0.5 | `__aarch64_swp8_acq` |
| 570.0 | 570.4 | 0.4 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 29.9 | 29.5 | -0.4 | `__aarch64_cas8_rel` |
| 916.6 | 916.9 | 0.3 | `<std::sys::pal::unix::time::Timespec>::now` |
| 212.5 | 212.2 | -0.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 137.0 | 137.3 | 0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 374.7 | 375.0 | 0.3 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 425.6 | 425.9 | 0.3 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 229.8 | 230.0 | 0.2 | `<alloc::raw_vec::RawVec<bacnet_types::constructed::read_access::PropertyReference>>::grow_one` |
| 117.4 | 117.7 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.4 | 87.6 | 0.2 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 17.9 | 18.2 | 0.2 | `alloc_perturb` |
| 95.6 | 95.8 | 0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,268.1 | 2,331.1 | 63.0 | `library/core/src/ptr/mod.rs:573` |
| 1,700.2 | 1,700.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,663.9 | 1,664.0 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,605.6 | 1,583.6 | -22.0 | `library/core/src/ptr/mod.rs:847` |
| 1,500.0 | 1,501.5 | 1.5 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 750.0 | 750.8 | 0.8 | `./malloc/./malloc/malloc.c:4495` |
| 745.0 | 745.4 | 0.4 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | -0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 645.9 | 645.8 | -0.2 | `./malloc/./malloc/malloc.c:1336` |
| 621.5 | 621.7 | 0.2 | `./malloc/./malloc/malloc.c:4658` |
| 620.0 | 620.2 | 0.2 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 569.7 | 567.3 | -2.4 | `???:0` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 552.8 | 552.9 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 512.7 | 526.4 | 13.7 | `./malloc/./malloc/arena.c:147` |
| 500.0 | 500.5 | 0.5 | `./malloc/./malloc/malloc.c:4510` |
| 500.0 | 500.5 | 0.5 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.0 | 452.9 | 0.8 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 416.0 | 416.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 401.5 | 396.4 | -5.1 | `./malloc/./malloc/arena.c:153` |
| 396.5 | 396.5 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 383.0 | 383.1 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 353.1 | 352.9 | -0.2 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | -0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | -0.0 | `./malloc/./malloc/malloc.c:3395` |
| 342.1 | 342.1 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 342.1 | 342.7 | 0.7 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 294.7 | 294.7 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 293.8 | 293.8 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.0 | 292.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 283.0 | 283.0 | 0.0 | `library/core/src/result.rs:2178` |
| 279.1 | 279.1 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
