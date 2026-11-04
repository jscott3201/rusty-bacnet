# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,307.7 | 8,293.5 | -14.2 | 125.20 | `_int_free` |
| 6,107.3 | 6,107.3 | 0.0 | 113.00 | `malloc` |
| 4,880.2 | 4,875.0 | -5.2 | 116.00 | `free` |
| 3,889.0 | 3,879.6 | -9.4 | 167.93 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,607.7 | 2,484.1 | -123.6 | 13.06 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,021.9 | -0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,957.0 | 1,961.1 | 4.2 | 32.81 | `realloc` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.2 | 0.1 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,621.0 | 1,621.2 | 0.2 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,540.7 | 0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.2 | 1,234.6 | 0.4 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 919.5 | 919.2 | -0.3 | 13.30 | `_int_realloc` |
| 865.8 | 865.8 | 0.0 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 847.6 | 848.0 | 0.4 | 11.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 698.4 | 698.9 | 0.5 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 603.9 | 602.4 | -1.5 | 112.81 | `__aarch64_cas4_acq` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.1 | 570.8 | 0.6 | 114.02 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 477.3 | 475.7 | -1.6 | 90.71 | `__aarch64_swp4_rel` |
| 468.4 | 469.0 | 0.7 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 433.8 | 455.0 | 21.2 | 2.62 | `malloc_consolidate` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 426.4 | 427.0 | 0.7 | 1.32 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.6 | 389.9 | -20.7 | 9.53 | `_int_free_merge_chunk` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.4 | 398.4 | -0.0 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.4 | 379.6 | 0.2 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 379.1 | 380.3 | 1.2 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 357.6 | 357.9 | 0.3 | 11.18 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 338.4 | 338.5 | 0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 2,607.7 | 2,484.1 | -123.6 | `_int_malloc` |
| 329.3 | 299.5 | -29.7 | `_int_free_create_chunk` |
| 433.8 | 455.0 | 21.2 | `malloc_consolidate` |
| 103.1 | 82.1 | -21.0 | `_mid_memalign.isra.0` |
| 410.6 | 389.9 | -20.7 | `_int_free_merge_chunk` |
| 74.0 | 88.5 | 14.5 | `_int_free_maybe_consolidate.part.0` |
| 8,307.7 | 8,293.5 | -14.2 | `_int_free` |
| 157.0 | 168.0 | 11.0 | `__aarch64_swp8_acq` |
| 3,889.0 | 3,879.6 | -9.4 | `__GI_memcpy` |
| 4,880.2 | 4,875.0 | -5.2 | `free` |
| 4.9 | 0.0 | -4.9 | `_int_memalign` |
| 1,957.0 | 1,961.1 | 4.2 | `realloc` |
| 227.7 | 224.6 | -3.1 | `unlink_chunk.isra.0` |
| 18.0 | 20.0 | 2.1 | `alloc_perturb` |
| 477.3 | 475.7 | -1.6 | `__aarch64_swp4_rel` |
| 603.9 | 602.4 | -1.5 | `__aarch64_cas4_acq` |
| 379.1 | 380.3 | 1.2 | `<tokio::runtime::time::Driver>::park_internal` |
| 117.2 | 118.0 | 0.8 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 49.8 | 49.0 | -0.8 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 87.5 | 88.2 | 0.7 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 426.4 | 427.0 | 0.7 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 468.4 | 469.0 | 0.7 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 570.1 | 570.8 | 0.6 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 29.7 | 30.3 | 0.6 | `__aarch64_cas8_rel` |
| 125.8 | 126.3 | 0.6 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 698.4 | 698.9 | 0.5 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 212.6 | 213.1 | 0.4 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 1,234.2 | 1,234.6 | 0.4 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 847.6 | 848.0 | 0.4 | `<std::sys::pal::unix::time::Timespec>::now` |
| 0.7 | 1.1 | 0.4 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,321.1 | 2,321.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,707.2 | 1,707.3 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,647.9 | 1,648.1 | 0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,502.3 | 1,499.5 | -2.9 | `./malloc/./malloc/malloc.c:4518` |
| 1,497.6 | 1,497.6 | 0.1 | `library/core/src/ptr/mod.rs:847` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,095.0 | 1,095.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 751.2 | 749.7 | -1.4 | `./malloc/./malloc/malloc.c:4495` |
| 745.9 | 744.6 | -1.2 | `./malloc/./malloc/malloc.c:4527` |
| 734.0 | 734.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 664.0 | 664.0 | 0.0 | `library/core/src/result.rs:0` |
| 645.3 | 641.2 | -4.1 | `./malloc/./malloc/malloc.c:1336` |
| 622.2 | 621.4 | -0.8 | `./malloc/./malloc/malloc.c:4658` |
| 620.7 | 619.7 | -1.0 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 563.8 | 571.8 | 8.0 | `???:0` |
| 536.9 | 537.1 | 0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 513.5 | 510.6 | -2.9 | `./malloc/./malloc/arena.c:147` |
| 500.8 | 499.8 | -1.0 | `./malloc/./malloc/malloc.c:4505` |
| 500.8 | 499.8 | -1.0 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.8 | 453.3 | 0.6 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | -0.0 | `library/core/src/cmp.rs:1051` |
| 412.0 | 412.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 400.2 | 401.9 | 1.7 | `./malloc/./malloc/arena.c:153` |
| 396.6 | 396.8 | 0.1 | `library/core/src/num/uint_macros.rs:2652` |
| 384.2 | 384.4 | 0.2 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.8 | 353.1 | 0.3 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.8 | 343.5 | 0.6 | `./malloc/./malloc/malloc.c:3186` |
| 342.1 | 342.3 | 0.2 | `library/core/src/ptr/mod.rs:1963` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 292.0 | 292.1 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.8 | 291.9 | 0.1 | `library/core/src/sync/atomic.rs:4083` |
| 280.0 | 280.0 | 0.0 | `library/core/src/result.rs:2178` |
| 270.0 | 270.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 267.1 | 267.1 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
