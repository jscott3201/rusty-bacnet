# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,207.9 | 8,208.5 | 0.6 | 125.98 | `_int_free` |
| 6,101.0 | 6,101.0 | -0.0 | 113.00 | `malloc` |
| 4,482.8 | 4,482.8 | 0.0 | 116.00 | `free` |
| 3,826.4 | 3,827.0 | 0.6 | 161.97 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 3,004.0 | 3,004.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,749.1 | 2,757.5 | 8.4 | 13.97 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,738.0 | 1,738.0 | 0.0 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.1 | -0.0 | 329.43 | `__aarch64_ldadd8_relax` |
| 1,621.2 | 1,621.2 | -0.0 | 40.04 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.7 | 1,540.7 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 999.0 | 998.4 | -0.6 | 16.65 | `<std::sys::pal::unix::time::Timespec>::now` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 947.0 | 947.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 872.8 | 872.0 | -0.8 | 13.00 | `_int_realloc` |
| 865.7 | 865.6 | -0.1 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 647.8 | 647.7 | -0.0 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 603.3 | 603.2 | -0.1 | 10.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 565.0 | 565.0 | 0.0 | 113.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 506.5 | 503.2 | -3.3 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 481.6 | 481.6 | 0.0 | 10.50 | `_int_free_merge_chunk` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.7 | 468.6 | -0.0 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.7 | 409.7 | -0.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.7 | 384.7 | -0.0 | 12.02 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 374.0 | 373.7 | -0.3 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 369.2 | 368.9 | -0.3 | 10.50 | `_int_free_create_chunk` |
| 364.3 | 364.2 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 2,749.1 | 2,757.5 | 8.4 | `_int_malloc` |
| 179.6 | 183.4 | 3.8 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 506.5 | 503.2 | -3.3 | `<tokio::runtime::time::Driver>::park_internal` |
| 253.5 | 252.0 | -1.5 | `unlink_chunk.isra.0` |
| 872.8 | 872.0 | -0.8 | `_int_realloc` |
| 8,207.9 | 8,208.5 | 0.6 | `_int_free` |
| 999.0 | 998.4 | -0.6 | `<std::sys::pal::unix::time::Timespec>::now` |
| 3,826.4 | 3,827.0 | 0.6 | `__GI_memcpy` |
| 137.2 | 136.7 | -0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 110.6 | 110.3 | -0.3 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 374.0 | 373.7 | -0.3 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 369.2 | 368.9 | -0.3 | `_int_free_create_chunk` |
| 95.7 | 95.4 | -0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 101.1 | 101.3 | 0.2 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 82.8 | 83.0 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 34.8 | 34.7 | -0.2 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 338.3 | 338.2 | -0.1 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 108.2 | 108.1 | -0.1 | `tokio::runtime::task::raw::poll::<core::pin::Pin<alloc::boxed::Box<bacnet_server::server::audit_recipient::spawn_owned<<bacnet_server::server::BACnetServer<bacn` |
| 603.3 | 603.2 | -0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 108.0 | 108.1 | 0.1 | `tokio::runtime::task::waker::drop_waker` |
| 364.3 | 364.2 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 1.4 | 1.4 | -0.1 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 91.3 | 91.2 | -0.1 | `<tokio::runtime::time::handle::Handle>::reregister` |
| 255.7 | 255.6 | -0.1 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 122.7 | 122.6 | -0.1 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 284.7 | 284.8 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 76.7 | 76.6 | -0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 164.2 | 164.3 | 0.1 | `tokio::runtime::task::waker::clone_waker` |
| 865.7 | 865.6 | -0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 481.6 | 481.6 | 0.0 | `_int_free_merge_chunk` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,131.1 | 2,131.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,701.2 | 1,701.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,636.0 | 1,635.9 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,624.4 | 1,624.3 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,511.7 | 1,512.0 | 0.3 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 755.9 | 756.0 | 0.1 | `./malloc/./malloc/malloc.c:4495` |
| 750.9 | 751.0 | 0.1 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | -0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 653.8 | 654.0 | 0.2 | `./malloc/./malloc/malloc.c:1336` |
| 624.9 | 625.0 | 0.1 | `./malloc/./malloc/malloc.c:4547` |
| 609.0 | 609.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 574.0 | 574.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 503.9 | 504.0 | 0.1 | `./malloc/./malloc/malloc.c:4510` |
| 503.9 | 504.0 | 0.1 | `./malloc/./malloc/malloc.c:4505` |
| 487.8 | 487.8 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 451.6 | 454.2 | 2.6 | `./malloc/./malloc/malloc.c:3163` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 415.0 | 415.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 410.8 | 410.7 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 409.0 | 409.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 392.6 | 392.6 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 361.1 | 361.1 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 349.1 | 349.0 | -0.1 | `./malloc/./malloc/malloc.c:1461` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 340.9 | 343.2 | 2.3 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 293.0 | 293.0 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 289.5 | 289.5 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 283.0 | 283.0 | 0.0 | `library/core/src/result.rs:2178` |
| 270.0 | 270.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 268.9 | 268.9 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.2 | 267.2 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
