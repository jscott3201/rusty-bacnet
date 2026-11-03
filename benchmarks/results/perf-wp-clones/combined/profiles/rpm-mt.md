# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,301.5 | 8,325.5 | 24.0 | 125.12 | `_int_free` |
| 6,107.3 | 6,107.2 | -0.0 | 113.00 | `malloc` |
| 4,868.5 | 4,868.8 | 0.3 | 116.00 | `free` |
| 3,773.4 | 3,783.5 | 10.1 | 160.84 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,553.0 | 2,684.0 | 131.0 | 13.05 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.0 | -0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,956.0 | 1,954.6 | -1.3 | 32.77 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,647.1 | -0.0 | 329.42 | `__aarch64_ldadd8_relax` |
| 1,620.9 | 1,620.8 | -0.1 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.5 | 1,540.4 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.5 | 1,234.6 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 947.0 | 947.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 921.2 | 908.3 | -12.9 | 13.17 | `_int_realloc` |
| 916.5 | 916.4 | -0.1 | 13.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.7 | 865.6 | -0.1 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 647.6 | 647.6 | -0.1 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 604.3 | 605.3 | 0.9 | 112.89 | `__aarch64_cas4_acq` |
| 603.2 | 603.5 | 0.3 | 10.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 571.2 | 570.9 | -0.3 | 114.24 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 477.8 | 478.7 | 0.9 | 90.79 | `__aarch64_swp4_rel` |
| 468.9 | 468.1 | -0.8 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 441.6 | 440.6 | -1.0 | 2.68 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 426.2 | 425.5 | -0.7 | 1.34 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 410.0 | 433.7 | 23.8 | 9.51 | `_int_free_merge_chunk` |
| 409.7 | 409.6 | -0.1 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.1 | 378.7 | -0.5 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 375.0 | 374.5 | -0.5 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 2,553.0 | 2,684.0 | 131.0 | `_int_malloc` |
| 326.7 | 356.1 | 29.5 | `_int_free_create_chunk` |
| 130.9 | 156.7 | 25.8 | `_mid_memalign.isra.0` |
| 8,301.5 | 8,325.5 | 24.0 | `_int_free` |
| 410.0 | 433.7 | 23.8 | `_int_free_merge_chunk` |
| 921.2 | 908.3 | -12.9 | `_int_realloc` |
| 209.8 | 221.7 | 11.9 | `unlink_chunk.isra.0` |
| 3,773.4 | 3,783.5 | 10.1 | `__GI_memcpy` |
| 11.3 | 17.3 | 6.0 | `_int_memalign` |
| 78.6 | 84.3 | 5.7 | `_int_free_maybe_consolidate.part.0` |
| 161.1 | 159.2 | -1.9 | `__aarch64_swp8_acq` |
| 1,956.0 | 1,954.6 | -1.3 | `realloc` |
| 441.6 | 440.6 | -1.0 | `malloc_consolidate` |
| 477.8 | 478.7 | 0.9 | `__aarch64_swp4_rel` |
| 604.3 | 605.3 | 0.9 | `__aarch64_cas4_acq` |
| 468.9 | 468.1 | -0.8 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 426.2 | 425.5 | -0.7 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 93.2 | 92.7 | -0.5 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 379.1 | 378.7 | -0.5 | `<tokio::runtime::time::Driver>::park_internal` |
| 375.0 | 374.5 | -0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 83.1 | 83.5 | 0.4 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 101.5 | 101.9 | 0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 29.6 | 30.0 | 0.4 | `__aarch64_cas8_rel` |
| 364.4 | 364.0 | -0.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 76.7 | 76.4 | -0.4 | `<tokio::time::interval::Interval>::poll_tick` |
| 4,868.5 | 4,868.8 | 0.3 | `free` |
| 603.2 | 603.5 | 0.3 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 571.2 | 570.9 | -0.3 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 95.4 | 95.7 | 0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 136.9 | 137.1 | 0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,131.1 | 2,131.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,704.2 | 1,704.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,664.0 | 1,663.9 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,606.6 | 1,606.6 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,501.4 | 1,505.6 | 4.2 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 750.7 | 752.8 | 2.1 | `./malloc/./malloc/malloc.c:4495` |
| 745.4 | 747.7 | 2.3 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | -0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 645.8 | 652.1 | 6.3 | `./malloc/./malloc/malloc.c:1336` |
| 621.7 | 622.7 | 1.0 | `./malloc/./malloc/malloc.c:4658` |
| 620.3 | 622.2 | 1.9 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 566.9 | 567.4 | 0.5 | `???:0` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 552.8 | 552.8 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 504.0 | 503.6 | -0.3 | `./malloc/./malloc/arena.c:147` |
| 500.5 | 501.9 | 1.4 | `./malloc/./malloc/malloc.c:4510` |
| 500.5 | 501.9 | 1.4 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 453.2 | 452.8 | -0.4 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 421.0 | 421.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 416.0 | 416.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 404.8 | 405.5 | 0.7 | `./malloc/./malloc/arena.c:153` |
| 396.6 | 396.6 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 389.1 | 389.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 385.0 | 385.0 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 353.0 | 352.2 | -0.8 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 343.4 | 342.8 | -0.6 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 294.7 | 294.7 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 293.9 | 293.8 | -0.1 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.0 | 292.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 287.1 | 287.1 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
