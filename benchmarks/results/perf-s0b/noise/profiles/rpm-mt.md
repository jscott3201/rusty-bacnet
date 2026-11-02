# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,305.2 | 8,305.8 | 0.6 | 125.21 | `_int_free` |
| 6,107.3 | 6,107.3 | -0.0 | 113.00 | `malloc` |
| 4,866.5 | 4,877.8 | 11.3 | 116.00 | `free` |
| 3,776.5 | 3,778.7 | 2.1 | 160.92 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | -0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,595.1 | 2,567.3 | -27.7 | 13.04 | `_int_malloc` |
| 2,122.0 | 2,122.0 | -0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.1 | 0.1 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,956.1 | 1,955.7 | -0.4 | 32.81 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.0 | 1,647.1 | 0.1 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,620.9 | 1,620.9 | 0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.5 | 1,540.5 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.5 | 1,234.6 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 947.0 | 947.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::clone::Clone>::clone` |
| 916.6 | 916.4 | -0.2 | 13.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 914.8 | 916.5 | 1.7 | 13.18 | `_int_realloc` |
| 865.7 | 865.6 | -0.2 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | -0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 647.8 | 647.6 | -0.1 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 604.3 | 602.8 | -1.5 | 112.89 | `__aarch64_cas4_acq` |
| 603.2 | 603.3 | 0.1 | 10.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.7 | 570.3 | -0.4 | 114.14 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 477.8 | 476.3 | -1.5 | 90.80 | `__aarch64_swp4_rel` |
| 467.8 | 468.6 | 0.9 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.7 | 445.9 | 5.2 | 2.73 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 426.1 | 426.2 | 0.1 | 1.34 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 420.2 | 403.2 | -17.0 | 9.82 | `_int_free_merge_chunk` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.8 | 409.7 | -0.1 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.0 | 379.2 | 0.2 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 374.3 | 374.9 | 0.6 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 123.5 | 86.8 | -36.7 | `_mid_memalign.isra.0` |
| 2,595.1 | 2,567.3 | -27.7 | `_int_malloc` |
| 420.2 | 403.2 | -17.0 | `_int_free_merge_chunk` |
| 334.0 | 319.2 | -14.8 | `_int_free_create_chunk` |
| 4,866.5 | 4,877.8 | 11.3 | `free` |
| 85.5 | 77.0 | -8.6 | `_int_free_maybe_consolidate.part.0` |
| 9.6 | 1.1 | -8.5 | `_int_memalign` |
| 440.7 | 445.9 | 5.2 | `malloc_consolidate` |
| 225.8 | 220.8 | -5.0 | `unlink_chunk.isra.0` |
| 3,776.5 | 3,778.7 | 2.1 | `__GI_memcpy` |
| 914.8 | 916.5 | 1.7 | `_int_realloc` |
| 477.8 | 476.3 | -1.5 | `__aarch64_swp4_rel` |
| 604.3 | 602.8 | -1.5 | `__aarch64_cas4_acq` |
| 163.5 | 162.2 | -1.3 | `__aarch64_swp8_acq` |
| 18.7 | 17.8 | -0.9 | `alloc_perturb` |
| 467.8 | 468.6 | 0.9 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 374.3 | 374.9 | 0.6 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 8,305.2 | 8,305.8 | 0.6 | `_int_free` |
| 101.8 | 101.2 | -0.6 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 76.3 | 76.9 | 0.5 | `<tokio::time::interval::Interval>::poll_tick` |
| 570.7 | 570.3 | -0.4 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 29.4 | 29.8 | 0.4 | `__aarch64_cas8_rel` |
| 117.2 | 117.6 | 0.4 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 83.2 | 82.9 | -0.4 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 1,956.1 | 1,955.7 | -0.4 | `realloc` |
| 127.1 | 126.7 | -0.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 87.3 | 87.6 | 0.4 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 92.6 | 92.9 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 197.6 | 197.9 | 0.2 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 916.6 | 916.4 | -0.2 | `<std::sys::pal::unix::time::Timespec>::now` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,131.1 | 2,131.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,704.2 | 1,704.2 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,663.9 | 1,664.0 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,605.6 | 1,605.6 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,502.4 | 1,503.0 | 0.5 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,090.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 751.2 | 751.5 | 0.3 | `./malloc/./malloc/malloc.c:4495` |
| 745.5 | 746.2 | 0.7 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 736.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 711.1 | 0.0 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 645.5 | 644.5 | -0.9 | `./malloc/./malloc/malloc.c:1336` |
| 622.1 | 622.1 | 0.1 | `./malloc/./malloc/malloc.c:4658` |
| 620.4 | 621.0 | 0.6 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 571.2 | 566.4 | -4.8 | `???:0` |
| 565.0 | 565.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 552.8 | 552.9 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 501.2 | 512.1 | 10.9 | `./malloc/./malloc/arena.c:147` |
| 500.8 | 501.0 | 0.2 | `./malloc/./malloc/malloc.c:4510` |
| 500.8 | 501.0 | 0.2 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | -0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 453.6 | 452.7 | -0.9 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 421.0 | 421.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 416.0 | 416.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 405.9 | 401.0 | -4.9 | `./malloc/./malloc/arena.c:153` |
| 396.6 | 396.6 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 389.1 | 389.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 385.0 | 385.0 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.6 | 352.4 | -0.2 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 343.6 | 342.5 | -1.0 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 294.7 | 294.7 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 293.8 | 293.8 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.0 | 291.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 287.1 | 287.1 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
