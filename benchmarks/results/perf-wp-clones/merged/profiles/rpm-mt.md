# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,313.9 | 8,312.1 | -1.8 | 125.32 | `_int_free` |
| 6,107.4 | 6,107.5 | 0.1 | 113.00 | `malloc` |
| 4,879.5 | 4,886.1 | 6.5 | 116.00 | `free` |
| 3,892.2 | 3,893.0 | 0.8 | 168.06 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,627.1 | 2,617.1 | -10.0 | 13.43 | `_int_malloc` |
| 2,121.9 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.0 | -0.0 | 30.98 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,958.8 | 1,956.3 | -2.6 | 33.01 | `realloc` |
| 1,752.0 | 1,752.0 | 0.0 | 4.03 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.2 | 1,647.4 | 0.2 | 329.44 | `__aarch64_ldadd8_relax` |
| 1,621.1 | 1,622.2 | 1.1 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,541.2 | 0.6 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.3 | 1,234.5 | 0.2 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 917.4 | 922.1 | 4.7 | 13.16 | `<std::sys::pal::unix::time::Timespec>::now` |
| 911.2 | 913.1 | 1.9 | 13.15 | `_int_realloc` |
| 865.9 | 866.5 | 0.7 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 663.8 | 664.6 | 0.8 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.6 | 637.4 | 1.7 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 604.4 | 606.9 | 2.5 | 112.91 | `__aarch64_cas4_acq` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.2 | 569.8 | -0.4 | 114.03 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 477.8 | 480.2 | 2.5 | 90.81 | `__aarch64_swp4_rel` |
| 468.4 | 469.5 | 1.1 | 10.00 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 439.7 | 435.5 | -4.2 | 2.71 | `malloc_consolidate` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 427.1 | 430.0 | 2.9 | 1.35 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 418.2 | 419.7 | 1.5 | 9.74 | `_int_free_merge_chunk` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.9 | 410.7 | 0.7 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.5 | 383.0 | 3.4 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 374.9 | 376.1 | 1.2 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.3 | 364.4 | 0.2 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 136.9 | 158.6 | 21.7 | `_mid_memalign.isra.0` |
| 225.5 | 213.2 | -12.3 | `unlink_chunk.isra.0` |
| 2,627.1 | 2,617.1 | -10.0 | `_int_malloc` |
| 4,879.5 | 4,886.1 | 6.5 | `free` |
| 12.7 | 17.7 | 5.1 | `_int_memalign` |
| 162.5 | 157.8 | -4.7 | `__aarch64_swp8_acq` |
| 917.4 | 922.1 | 4.7 | `<std::sys::pal::unix::time::Timespec>::now` |
| 439.7 | 435.5 | -4.2 | `malloc_consolidate` |
| 379.5 | 383.0 | 3.4 | `<tokio::runtime::time::Driver>::park_internal` |
| 80.9 | 77.6 | -3.3 | `_int_free_maybe_consolidate.part.0` |
| 213.3 | 216.2 | 2.9 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 427.1 | 430.0 | 2.9 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 1,958.8 | 1,956.3 | -2.6 | `realloc` |
| 604.4 | 606.9 | 2.5 | `__aarch64_cas4_acq` |
| 477.8 | 480.2 | 2.5 | `__aarch64_swp4_rel` |
| 333.7 | 336.1 | 2.4 | `_int_free_create_chunk` |
| 128.2 | 130.5 | 2.3 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 911.2 | 913.1 | 1.9 | `_int_realloc` |
| 2.0 | 3.8 | 1.8 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 49.1 | 47.3 | -1.8 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 8,313.9 | 8,312.1 | -1.8 | `_int_free` |
| 357.6 | 359.4 | 1.7 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 635.6 | 637.4 | 1.7 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 105.6 | 107.1 | 1.5 | `<tokio::runtime::scheduler::multi_thread::worker::Core>::maintenance` |
| 418.2 | 419.7 | 1.5 | `_int_free_merge_chunk` |
| 1.3 | 2.7 | 1.4 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 17.7 | 16.4 | -1.4 | `alloc_perturb` |
| 198.1 | 199.4 | 1.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 374.9 | 376.1 | 1.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 91.1 | 92.3 | 1.2 | `<tokio::runtime::time::handle::Handle>::reregister` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,331.1 | 2,331.1 | 0.1 | `library/core/src/ptr/mod.rs:573` |
| 1,700.3 | 1,700.4 | 0.1 | `library/core/src/sync/atomic.rs:4029` |
| 1,664.1 | 1,664.9 | 0.7 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,584.7 | 1,585.4 | 0.7 | `library/core/src/ptr/mod.rs:847` |
| 1,503.7 | 1,502.4 | -1.3 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 751.9 | 751.3 | -0.6 | `./malloc/./malloc/malloc.c:4495` |
| 746.6 | 746.1 | -0.4 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | 0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 650.5 | 650.5 | -0.1 | `./malloc/./malloc/malloc.c:1336` |
| 622.3 | 622.1 | -0.2 | `./malloc/./malloc/malloc.c:4658` |
| 621.2 | 620.9 | -0.2 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 569.9 | 568.6 | -1.3 | `???:0` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 553.1 | 554.6 | 1.5 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 518.7 | 520.6 | 1.9 | `./malloc/./malloc/arena.c:147` |
| 501.3 | 500.8 | -0.4 | `./malloc/./malloc/malloc.c:4505` |
| 501.3 | 500.8 | -0.4 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.1 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.1 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.5 | 452.4 | -0.1 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 416.0 | 416.2 | 0.2 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 400.3 | 398.9 | -1.4 | `./malloc/./malloc/arena.c:153` |
| 396.7 | 397.4 | 0.7 | `library/core/src/num/uint_macros.rs:2652` |
| 383.2 | 384.2 | 1.0 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.4 | 352.5 | 0.1 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.4 | 342.4 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 342.3 | 342.7 | 0.5 | `library/core/src/ptr/mod.rs:1963` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 294.8 | 295.7 | 0.8 | `library/core/src/sync/atomic.rs:4083` |
| 294.1 | 295.6 | 1.6 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.1 | 293.0 | 0.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 283.0 | 283.0 | 0.0 | `library/core/src/result.rs:2178` |
| 279.1 | 279.2 | 0.1 | `library/core/src/ptr/mut_ptr.rs:970` |
