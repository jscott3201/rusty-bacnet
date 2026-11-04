# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,297.8 | 8,318.8 | 21.0 | 124.99 | `_int_free` |
| 6,108.0 | 6,107.3 | -0.7 | 113.00 | `malloc` |
| 4,882.7 | 4,897.6 | 14.9 | 116.00 | `free` |
| 3,886.2 | 3,891.0 | 4.8 | 167.79 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,523.8 | 2,670.4 | 146.6 | 13.08 | `_int_malloc` |
| 2,121.7 | 2,122.0 | 0.3 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.5 | 0.5 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,955.0 | 1,969.2 | 14.2 | 32.80 | `realloc` |
| 1,751.9 | 1,752.0 | 0.1 | 4.15 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.1 | 0.1 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,621.0 | 1,620.9 | -0.1 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,540.5 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.6 | 1,234.4 | -0.2 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 924.4 | 920.1 | -4.4 | 13.18 | `_int_realloc` |
| 917.1 | 846.6 | -70.5 | 13.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.8 | 865.8 | 0.1 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.5 | 689.6 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.5 | 698.6 | 63.1 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 603.5 | 604.1 | 0.6 | 112.76 | `__aarch64_cas4_acq` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.7 | 570.3 | -0.4 | 114.14 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 476.9 | 477.5 | 0.6 | 90.66 | `__aarch64_swp4_rel` |
| 468.5 | 469.2 | 0.6 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 451.6 | 437.2 | -14.3 | 2.74 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 426.3 | 426.0 | -0.2 | 1.35 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.6 | 398.7 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 397.6 | 436.9 | 39.2 | 9.24 | `_int_free_merge_chunk` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.4 | 378.9 | -0.4 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 379.3 | 379.6 | 0.2 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 357.5 | 357.4 | -0.1 | 11.17 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 338.4 | 338.3 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 170.6 | 0.0 | -170.6 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}` |
| 2,523.8 | 2,670.4 | 146.6 | `_int_malloc` |
| 917.1 | 846.6 | -70.5 | `<std::sys::pal::unix::time::Timespec>::now` |
| 635.5 | 698.6 | 63.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 58.1 | 0.0 | -58.1 | `core::ptr::drop_glue::<alloc::vec::into_iter::IntoIter<bacnet_server::server::segmented_receive::ExpiredRequest>>` |
| 82.1 | 135.2 | 53.1 | `_mid_memalign.isra.0` |
| 309.4 | 356.0 | 46.7 | `_int_free_create_chunk` |
| 397.6 | 436.9 | 39.2 | `_int_free_merge_chunk` |
| 8,297.8 | 8,318.8 | 21.0 | `_int_free` |
| 172.9 | 152.8 | -20.1 | `clock_gettime@@GLIBC_2.17` |
| 18.0 | 0.0 | -18.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::reap_expired_requests::{closure#0}>` |
| 225.3 | 240.3 | 14.9 | `unlink_chunk.isra.0` |
| 4,882.7 | 4,897.6 | 14.9 | `free` |
| 451.6 | 437.2 | -14.3 | `malloc_consolidate` |
| 1,955.0 | 1,969.2 | 14.2 | `realloc` |
| 0.0 | 12.3 | 12.3 | `_int_memalign` |
| 3,886.2 | 3,891.0 | 4.8 | `__GI_memcpy` |
| 924.4 | 920.1 | -4.4 | `_int_realloc` |
| 79.5 | 82.3 | 2.7 | `_int_free_maybe_consolidate.part.0` |
| 30.2 | 29.0 | -1.2 | `__aarch64_cas8_rel` |
| 229.0 | 230.0 | 1.0 | `<alloc::raw_vec::RawVec<bacnet_types::constructed::read_access::PropertyReference>>::grow_one` |
| 6,108.0 | 6,107.3 | -0.7 | `malloc` |
| 468.5 | 469.2 | 0.6 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 476.9 | 477.5 | 0.6 | `__aarch64_swp4_rel` |
| 603.5 | 604.1 | 0.6 | `__aarch64_cas4_acq` |
| 2,022.0 | 2,022.5 | 0.5 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 164.3 | 164.7 | 0.5 | `__aarch64_swp8_acq` |
| 379.4 | 378.9 | -0.4 | `<tokio::runtime::time::Driver>::park_internal` |
| 18.6 | 19.0 | 0.4 | `alloc_perturb` |
| 570.7 | 570.3 | -0.4 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,320.1 | 2,321.1 | 1.0 | `library/core/src/ptr/mod.rs:573` |
| 1,707.2 | 1,707.3 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,647.9 | 1,648.0 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,535.6 | 1,497.5 | -38.1 | `library/core/src/ptr/mod.rs:847` |
| 1,499.9 | 1,505.5 | 5.7 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,095.0 | 1,095.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 750.0 | 752.8 | 2.8 | `./malloc/./malloc/malloc.c:4495` |
| 745.0 | 746.3 | 1.3 | `./malloc/./malloc/malloc.c:4527` |
| 734.0 | 734.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 664.0 | 664.0 | -0.0 | `library/core/src/result.rs:0` |
| 644.9 | 648.8 | 3.9 | `./malloc/./malloc/malloc.c:1336` |
| 621.4 | 622.6 | 1.2 | `./malloc/./malloc/malloc.c:4658` |
| 619.9 | 620.6 | 0.8 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 569.8 | 572.0 | 2.2 | `???:0` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 536.8 | 536.8 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 519.6 | 543.3 | 23.7 | `./malloc/./malloc/arena.c:147` |
| 500.0 | 501.9 | 1.9 | `./malloc/./malloc/malloc.c:4510` |
| 500.0 | 501.9 | 1.9 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.1 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.1 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 453.4 | 453.4 | -0.0 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 412.0 | 412.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 398.9 | 392.7 | -6.3 | `./malloc/./malloc/arena.c:153` |
| 396.6 | 396.6 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 384.2 | 384.3 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.8 | 352.9 | 0.1 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 343.5 | 343.5 | 0.1 | `./malloc/./malloc/malloc.c:3186` |
| 342.1 | 342.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 293.9 | 259.8 | -34.2 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.0 | 292.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.7 | 291.7 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 280.0 | 280.0 | 0.0 | `library/core/src/result.rs:2178` |
| 271.1 | 267.1 | -4.0 | `library/core/src/ptr/mut_ptr.rs:970` |
