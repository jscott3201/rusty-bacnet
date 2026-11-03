# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,223.5 | 8,243.3 | 19.8 | 126.23 | `_int_free` |
| 6,101.0 | 6,101.0 | 0.0 | 113.00 | `malloc` |
| 4,483.2 | 4,484.2 | 1.0 | 116.00 | `free` |
| 3,832.1 | 3,838.8 | 6.7 | 162.23 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 3,004.0 | 3,004.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,762.0 | 2,684.5 | -77.5 | 14.23 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,738.0 | 1,738.0 | 0.0 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.0 | 1,647.0 | -0.0 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | -0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 997.8 | 997.6 | -0.2 | 16.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 947.0 | 947.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 865.6 | 865.4 | -0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 864.6 | 854.9 | -9.7 | 13.00 | `_int_realloc` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 647.5 | 647.5 | -0.0 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 602.8 | 602.7 | -0.2 | 10.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 565.0 | 565.0 | 0.0 | 113.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 502.1 | 500.7 | -1.4 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 481.6 | 481.2 | -0.4 | 10.50 | `_int_free_merge_chunk` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.1 | 468.2 | 0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.5 | 409.5 | -0.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.5 | 384.5 | -0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 373.3 | 373.4 | 0.1 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 369.6 | 367.9 | -1.7 | 10.50 | `_int_free_create_chunk` |
| 364.2 | 364.0 | -0.2 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 2,762.0 | 2,684.5 | -77.5 | `_int_malloc` |
| 225.1 | 188.5 | -36.7 | `unlink_chunk.isra.0` |
| 8,223.5 | 8,243.3 | 19.8 | `_int_free` |
| 864.6 | 854.9 | -9.7 | `_int_realloc` |
| 3,832.1 | 3,838.8 | 6.7 | `__GI_memcpy` |
| 184.0 | 185.9 | 1.9 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 369.6 | 367.9 | -1.7 | `_int_free_create_chunk` |
| 502.1 | 500.7 | -1.4 | `<tokio::runtime::time::Driver>::park_internal` |
| 4,483.2 | 4,484.2 | 1.0 | `free` |
| 481.6 | 481.2 | -0.4 | `_int_free_merge_chunk` |
| 93.0 | 92.8 | -0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 101.4 | 101.6 | 0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 137.1 | 136.9 | -0.2 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 997.8 | 997.6 | -0.2 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.6 | 865.4 | -0.2 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 602.8 | 602.7 | -0.2 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 364.2 | 364.0 | -0.2 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.1 | 468.2 | 0.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 110.2 | 110.1 | -0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 95.7 | 95.6 | -0.1 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 373.3 | 373.4 | 0.1 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 108.1 | 108.0 | -0.1 | `tokio::runtime::task::waker::drop_waker` |
| 164.2 | 164.1 | -0.1 | `tokio::runtime::task::waker::clone_waker` |
| 34.6 | 34.6 | -0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 284.7 | 284.6 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 338.1 | 338.1 | -0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 117.7 | 117.7 | 0.0 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 76.5 | 76.5 | 0.0 | `<tokio::time::interval::Interval>::poll_tick` |
| 122.7 | 122.6 | -0.0 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 1.0 | 0.9 | -0.0 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,131.1 | 2,131.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,701.2 | 1,701.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,635.8 | 1,635.8 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,624.1 | 1,624.1 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,514.8 | 1,518.4 | 3.7 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 757.4 | 759.2 | 1.8 | `./malloc/./malloc/malloc.c:4495` |
| 752.4 | 754.2 | 1.8 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.0 | 711.0 | -0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 656.3 | 659.4 | 3.0 | `./malloc/./malloc/malloc.c:1336` |
| 626.2 | 627.7 | 1.5 | `./malloc/./malloc/malloc.c:4547` |
| 610.1 | 611.7 | 1.5 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 574.0 | 574.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 504.9 | 506.1 | 1.2 | `./malloc/./malloc/malloc.c:4510` |
| 504.9 | 506.1 | 1.2 | `./malloc/./malloc/malloc.c:4505` |
| 487.6 | 487.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.9 | 454.7 | 1.8 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 415.0 | 415.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 410.5 | 410.5 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 409.0 | 409.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 392.1 | 392.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 361.0 | 361.0 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.3 | 347.4 | -0.9 | `./malloc/./malloc/malloc.c:1461` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.2 | 343.7 | 1.5 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 293.0 | 293.0 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 289.3 | 289.3 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 283.0 | 283.0 | 0.0 | `library/core/src/result.rs:2178` |
| 270.0 | 270.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 268.9 | 268.9 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.1 | 267.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
