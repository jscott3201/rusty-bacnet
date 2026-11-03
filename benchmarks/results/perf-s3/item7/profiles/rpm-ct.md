# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,180.3 | 8,204.3 | 24.1 | 125.50 | `_int_free` |
| 6,101.0 | 6,101.0 | 0.0 | 113.00 | `malloc` |
| 4,481.1 | 4,473.0 | -8.0 | 116.00 | `free` |
| 3,893.1 | 3,925.6 | 32.5 | 168.00 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 3,004.0 | 3,004.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,781.8 | 2,771.1 | -10.7 | 13.00 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,748.5 | 1,749.1 | 0.6 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.1 | 0.0 | 329.42 | `__aarch64_ldadd8_relax` |
| 1,621.0 | 1,620.9 | -0.1 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,540.5 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 998.1 | 998.0 | -0.1 | 16.65 | `<std::sys::pal::unix::time::Timespec>::now` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 899.9 | 861.8 | -38.0 | 13.50 | `_int_realloc` |
| 865.6 | 865.5 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 701.9 | 635.0 | -66.8 | 9.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 663.6 | 663.6 | -0.0 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 565.0 | 565.0 | -0.0 | 113.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 503.1 | 502.7 | -0.4 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.7 | 468.9 | 0.1 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 436.7 | 477.6 | 40.9 | 9.50 | `_int_free_merge_chunk` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.6 | 409.5 | -0.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.5 | 384.5 | -0.0 | 12.02 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 373.4 | 373.9 | 0.5 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 369.2 | 312.3 | -56.9 | 17.48 | `unlink_chunk.isra.0` |
| 364.4 | 364.2 | -0.2 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 358.1 | 388.3 | 30.2 | 9.50 | `_int_free_create_chunk` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 701.9 | 635.0 | -66.8 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 369.2 | 312.3 | -56.9 | `unlink_chunk.isra.0` |
| 436.7 | 477.6 | 40.9 | `_int_free_merge_chunk` |
| 899.9 | 861.8 | -38.0 | `_int_realloc` |
| 3,893.1 | 3,925.6 | 32.5 | `__GI_memcpy` |
| 358.1 | 388.3 | 30.2 | `_int_free_create_chunk` |
| 8,180.3 | 8,204.3 | 24.1 | `_int_free` |
| 225.0 | 247.6 | 22.6 | `malloc_consolidate` |
| 42.0 | 28.0 | -14.0 | `core::ptr::drop_glue::<bacnet_network::response_route::ResponseRoute>` |
| 28.0 | 14.3 | -13.7 | `_int_free_maybe_consolidate.part.0` |
| 90.0 | 101.9 | 11.9 | `__aarch64_swp8_acq` |
| 2,781.8 | 2,771.1 | -10.7 | `_int_malloc` |
| 4,481.1 | 4,473.0 | -8.0 | `free` |
| 276.0 | 277.0 | 1.0 | `bacnet_encoding::apdu::decode_apdu` |
| 182.8 | 183.7 | 0.9 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 1,748.5 | 1,749.1 | 0.6 | `realloc` |
| 373.4 | 373.9 | 0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 503.1 | 502.7 | -0.4 | `<tokio::runtime::time::Driver>::park_internal` |
| 137.1 | 137.4 | 0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 117.9 | 117.5 | -0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 93.4 | 93.0 | -0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 87.9 | 87.6 | -0.3 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 101.5 | 101.2 | -0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 14.0 | 13.7 | -0.3 | `alloc_perturb` |
| 364.4 | 364.2 | -0.2 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 95.6 | 95.8 | 0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 468.7 | 468.9 | 0.1 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 51.2 | 51.4 | 0.1 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 43.9 | 44.0 | 0.1 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 865.6 | 865.5 | -0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,268.1 | 2,331.1 | 63.0 | `library/core/src/ptr/mod.rs:573` |
| 1,697.2 | 1,697.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,635.9 | 1,635.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,623.2 | 1,601.2 | -22.0 | `library/core/src/ptr/mod.rs:847` |
| 1,506.1 | 1,514.7 | 8.6 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 753.0 | 757.3 | 4.3 | `./malloc/./malloc/malloc.c:4495` |
| 748.0 | 751.2 | 3.2 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | -0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 644.0 | 653.7 | 9.6 | `./malloc/./malloc/malloc.c:1336` |
| 622.5 | 625.0 | 2.4 | `./malloc/./malloc/malloc.c:4547` |
| 608.5 | 610.4 | 1.8 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 574.0 | 574.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 502.0 | 504.9 | 2.9 | `./malloc/./malloc/malloc.c:4510` |
| 502.0 | 504.9 | 2.9 | `./malloc/./malloc/malloc.c:4505` |
| 487.7 | 487.7 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 454.5 | 454.0 | -0.5 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | -0.0 | `library/core/src/cmp.rs:1051` |
| 410.6 | 410.6 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 409.0 | 409.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 359.0 | 359.0 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 352.0 | 349.1 | -2.9 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 343.5 | 342.2 | -1.3 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 289.4 | 289.4 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 285.0 | 285.0 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 283.0 | 283.0 | 0.0 | `library/core/src/result.rs:2178` |
| 270.0 | 270.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 268.9 | 268.9 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.1 | 267.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 262.0 | 262.0 | -0.0 | `library/core/src/num/uint_macros.rs:3212` |
