# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,309.2 | 8,308.9 | -0.3 | 125.27 | `_int_free` |
| 6,107.4 | 6,107.4 | 0.1 | 113.00 | `malloc` |
| 4,875.2 | 4,882.9 | 7.6 | 116.00 | `free` |
| 3,781.9 | 3,772.5 | -9.4 | 161.08 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | -0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,599.7 | 2,574.1 | -25.5 | 13.37 | `_int_malloc` |
| 2,122.0 | 2,121.9 | -0.1 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,023.9 | 2,024.0 | 0.1 | 30.99 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,955.8 | 1,956.7 | 0.9 | 32.78 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.5 | 1,647.2 | -0.3 | 329.49 | `__aarch64_ldadd8_relax` |
| 1,622.3 | 1,621.5 | -0.8 | 40.07 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,541.3 | 1,540.8 | -0.4 | 20.02 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.6 | 1,234.2 | -0.4 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 947.0 | 947.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 922.8 | 920.3 | -2.5 | 13.23 | `<std::sys::pal::unix::time::Timespec>::now` |
| 911.8 | 920.7 | 8.9 | 13.18 | `_int_realloc` |
| 866.5 | 866.0 | -0.5 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.5 | -0.1 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 648.6 | 648.1 | -0.4 | 7.02 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 606.8 | 603.9 | -2.9 | 113.37 | `__aarch64_cas4_acq` |
| 605.2 | 604.2 | -1.0 | 10.25 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 571.0 | 570.0 | -0.9 | 114.19 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 479.9 | 477.3 | -2.7 | 91.23 | `__aarch64_swp4_rel` |
| 469.3 | 468.8 | -0.5 | 10.01 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 439.5 | 440.7 | 1.2 | 2.68 | `malloc_consolidate` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 429.4 | 428.7 | -0.7 | 1.36 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 423.7 | 409.2 | -14.5 | 9.85 | `_int_free_merge_chunk` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.5 | 410.2 | -0.3 | 5.02 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 382.7 | 382.0 | -0.7 | 4.12 | `<tokio::runtime::time::Driver>::park_internal` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 376.1 | 374.8 | -1.2 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 144.3 | 105.4 | -38.9 | `_mid_memalign.isra.0` |
| 2,599.7 | 2,574.1 | -25.5 | `_int_malloc` |
| 339.5 | 324.6 | -15.0 | `_int_free_create_chunk` |
| 423.7 | 409.2 | -14.5 | `_int_free_merge_chunk` |
| 208.4 | 218.0 | 9.6 | `unlink_chunk.isra.0` |
| 3,781.9 | 3,772.5 | -9.4 | `__GI_memcpy` |
| 14.4 | 5.4 | -9.0 | `_int_memalign` |
| 911.8 | 920.7 | 8.9 | `_int_realloc` |
| 4,875.2 | 4,882.9 | 7.6 | `free` |
| 83.3 | 79.6 | -3.6 | `_int_free_maybe_consolidate.part.0` |
| 216.5 | 213.3 | -3.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 606.8 | 603.9 | -2.9 | `__aarch64_cas4_acq` |
| 479.9 | 477.3 | -2.7 | `__aarch64_swp4_rel` |
| 922.8 | 920.3 | -2.5 | `<std::sys::pal::unix::time::Timespec>::now` |
| 127.2 | 124.8 | -2.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 376.1 | 374.8 | -1.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 439.5 | 440.7 | 1.2 | `malloc_consolidate` |
| 605.2 | 604.2 | -1.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 18.2 | 17.2 | -1.0 | `alloc_perturb` |
| 3.8 | 2.8 | -1.0 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 359.6 | 358.7 | -0.9 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 571.0 | 570.0 | -0.9 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 199.4 | 198.5 | -0.9 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 1,955.8 | 1,956.7 | 0.9 | `realloc` |
| 2.8 | 1.9 | -0.9 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 102.6 | 101.8 | -0.8 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 1,622.3 | 1,621.5 | -0.8 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 84.0 | 83.2 | -0.8 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 107.2 | 106.5 | -0.8 | `<tokio::runtime::scheduler::multi_thread::worker::Core>::maintenance` |
| 92.4 | 91.6 | -0.7 | `<tokio::runtime::time::handle::Handle>::reregister` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,131.1 | 2,131.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,704.4 | 1,704.3 | -0.1 | `library/core/src/sync/atomic.rs:4029` |
| 1,664.9 | 1,664.3 | -0.6 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,607.5 | 1,607.0 | -0.5 | `library/core/src/ptr/mod.rs:847` |
| 1,503.2 | 1,502.4 | -0.8 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 751.6 | 751.2 | -0.4 | `./malloc/./malloc/malloc.c:4495` |
| 746.2 | 745.5 | -0.7 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | 0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 649.7 | 643.9 | -5.8 | `./malloc/./malloc/malloc.c:1336` |
| 621.9 | 622.1 | 0.1 | `./malloc/./malloc/malloc.c:4658` |
| 621.0 | 620.3 | -0.7 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 568.3 | 566.6 | -1.7 | `???:0` |
| 565.0 | 565.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 554.7 | 553.7 | -1.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 510.5 | 519.8 | 9.2 | `./malloc/./malloc/arena.c:147` |
| 501.1 | 500.8 | -0.3 | `./malloc/./malloc/malloc.c:4510` |
| 501.1 | 500.8 | -0.3 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | -0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 453.3 | 452.8 | -0.5 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | -0.0 | `library/core/src/cmp.rs:1051` |
| 421.1 | 421.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 416.2 | 416.1 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 403.6 | 399.0 | -4.6 | `./malloc/./malloc/arena.c:153` |
| 397.4 | 397.0 | -0.5 | `library/core/src/num/uint_macros.rs:2652` |
| 389.8 | 389.8 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 386.3 | 385.6 | -0.6 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.2 | 352.8 | 0.6 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 343.2 | 342.9 | -0.3 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 295.9 | 295.0 | -0.8 | `library/std/src/sys/pal/unix/time.rs:107` |
| 295.7 | 295.2 | -0.5 | `library/core/src/sync/atomic.rs:4083` |
| 293.1 | 292.5 | -0.6 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 287.2 | 287.2 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
