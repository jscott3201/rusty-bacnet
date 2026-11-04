# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,182.8 | 8,182.8 | 0.0 | 125.67 | `_int_free` |
| 6,101.0 | 6,101.0 | 0.0 | 113.00 | `malloc` |
| 4,470.1 | 4,470.1 | 0.0 | 116.00 | `free` |
| 3,890.5 | 3,890.5 | 0.0 | 168.00 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 3,004.0 | 3,004.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,990.0 | 2,990.2 | 0.1 | 13.00 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,752.0 | 1,752.0 | 0.0 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.0 | 1,647.0 | -0.0 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,621.0 | 1,620.9 | -0.1 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,540.5 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 928.0 | 927.9 | -0.1 | 14.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 896.2 | 896.2 | -0.0 | 13.00 | `_int_realloc` |
| 865.5 | 865.6 | 0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 698.2 | 698.2 | -0.0 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 565.0 | 565.0 | -0.0 | 113.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 502.4 | 503.2 | 0.8 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.2 | 467.9 | -0.4 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 429.9 | 429.9 | 0.0 | 9.34 | `_int_free_merge_chunk` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.5 | 398.5 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 391.0 | 391.0 | 0.0 | 18.65 | `unlink_chunk.isra.0` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.6 | 384.5 | -0.1 | 12.02 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 378.2 | 378.3 | 0.1 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 354.8 | 354.8 | 0.0 | 9.34 | `_int_free_create_chunk` |
| 338.2 | 338.2 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 184.5 | 182.5 | -2.0 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 502.4 | 503.2 | 0.8 | `<tokio::runtime::time::Driver>::park_internal` |
| 468.2 | 467.9 | -0.4 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 101.2 | 101.5 | 0.3 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 82.9 | 83.1 | 0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 136.9 | 137.2 | 0.2 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 96.5 | 96.8 | 0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 117.2 | 117.4 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 92.9 | 92.7 | -0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 87.5 | 87.6 | 0.2 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 2,990.0 | 2,990.2 | 0.1 | `_int_malloc` |
| 1.2 | 1.1 | -0.1 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 91.1 | 90.9 | -0.1 | `<tokio::runtime::time::handle::Handle>::reregister` |
| 110.2 | 110.3 | 0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_internal` |
| 928.0 | 927.9 | -0.1 | `<std::sys::pal::unix::time::Timespec>::now` |
| 378.2 | 378.3 | 0.1 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 1,621.0 | 1,620.9 | -0.1 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 0.8 | 0.7 | -0.1 | `<core::pin::Pin<alloc::boxed::Box<bacnet_server::server::audit_recipient::spawn_owned<bacnet_server::server::lifecycle::intrinsic::run<bacnet_transport::bip::Bi` |
| 0.7 | 0.6 | -0.1 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 258.6 | 258.6 | -0.1 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 0.6 | 0.5 | -0.1 | `<bacnet_objects::database::ObjectDatabase>::find_by_type` |
| 0.5 | 0.5 | -0.1 | `tokio::runtime::task::raw::poll::<core::pin::Pin<alloc::boxed::Box<bacnet_server::server::audit_recipient::spawn_owned<bacnet_server::trend_log::run::{closure#0` |
| 81.6 | 81.5 | -0.1 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 384.6 | 384.5 | -0.1 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 255.5 | 255.5 | -0.1 | `<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle> as tokio::runtime::task::Schedule>::schedule` |
| 1,540.6 | 1,540.5 | -0.1 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 0.5 | 0.5 | -0.1 | `<core::pin::Pin<alloc::boxed::Box<bacnet_server::server::audit_recipient::spawn_owned<bacnet_server::trend_log::run::{closure#0}, <bacnet_server::server::BACnet` |
| 398.5 | 398.5 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 34.6 | 34.7 | 0.1 | `<tokio::runtime::scheduler::current_thread::Context>::park_yield` |
| 160.5 | 160.4 | -0.1 | `tokio::runtime::task::waker::wake_by_val` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,321.1 | 2,321.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,704.2 | 1,704.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,619.8 | 1,619.8 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,514.1 | 1,514.1 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,508.0 | 1,508.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,095.0 | 1,095.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 754.0 | 754.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 749.0 | 749.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 734.0 | 734.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 664.0 | 664.0 | -0.0 | `library/core/src/result.rs:0` |
| 644.0 | 644.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 623.3 | 623.4 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 609.7 | 609.7 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 574.0 | 574.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 502.7 | 502.7 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 502.7 | 502.7 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 471.6 | 471.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 453.0 | 453.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | -0.0 | `library/core/src/cmp.rs:1051` |
| 410.6 | 410.6 | -0.1 | `library/core/src/num/uint_macros.rs:2652` |
| 405.0 | 405.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 360.0 | 360.1 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 352.0 | 352.0 | -0.0 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 341.0 | 341.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 280.0 | 280.0 | 0.0 | `library/core/src/result.rs:2178` |
| 273.0 | 273.0 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 270.0 | 270.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 267.2 | 267.2 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.9 | 265.9 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 262.0 | 262.0 | -0.0 | `library/core/src/num/uint_macros.rs:3212` |
| 260.0 | 260.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:655` |
