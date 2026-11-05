# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,179.4 | 8,179.2 | -0.2 | 123.16 | `_int_free` |
| 5,999.5 | 5,999.3 | -0.2 | 111.00 | `malloc` |
| 4,806.9 | 4,807.3 | 0.4 | 114.00 | `free` |
| 3,842.2 | 3,836.0 | -6.2 | 165.86 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,120.0 | 3,120.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,947.0 | 2,947.0 | -0.0 | 112.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,566.7 | 2,599.9 | 33.1 | 13.04 | `_int_malloc` |
| 2,122.0 | 2,122.0 | -0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.0 | 0.1 | 30.99 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,957.4 | 1,956.1 | -1.3 | 32.89 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,627.1 | 1,627.1 | -0.0 | 325.42 | `__aarch64_ldadd8_relax` |
| 1,621.0 | 1,620.8 | -0.2 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,585.0 | 1,585.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,540.4 | -0.1 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.4 | 1,234.3 | -0.1 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 950.0 | 950.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 921.6 | 922.5 | 0.9 | 13.25 | `_int_realloc` |
| 865.6 | 865.7 | 0.1 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 847.2 | 724.6 | -122.6 | 11.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 820.0 | 820.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 711.0 | 711.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 698.7 | 698.4 | -0.3 | 8.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | -0.1 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 602.7 | 604.0 | 1.3 | 112.61 | `__aarch64_cas4_acq` |
| 560.7 | 560.6 | -0.1 | 112.14 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 532.0 | 736.0 | 204.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 476.1 | 477.4 | 1.3 | 90.51 | `__aarch64_swp4_rel` |
| 468.7 | 468.7 | 0.0 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.8 | 446.7 | 15.8 | 2.62 | `malloc_consolidate` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 425.1 | 424.9 | -0.2 | 1.34 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 399.4 | 410.1 | 10.7 | 9.32 | `_int_free_merge_chunk` |
| 398.5 | 398.4 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 384.0 | 384.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.3 | 379.5 | 0.3 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.8 | 378.6 | -0.2 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 357.6 | 293.4 | -64.2 | 11.17 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 338.4 | 338.3 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 234.0 | 0.0 | -234.0 | `<bacnet_time::host::HostClock>::now` |
| 532.0 | 736.0 | 204.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 847.2 | 724.6 | -122.6 | `<std::sys::pal::unix::time::Timespec>::now` |
| 357.6 | 293.4 | -64.2 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 2,566.7 | 2,599.9 | 33.1 | `_int_malloc` |
| 82.3 | 104.9 | 22.6 | `_mid_memalign.isra.0` |
| 152.9 | 132.8 | -20.1 | `clock_gettime@@GLIBC_2.17` |
| 430.8 | 446.7 | 15.8 | `malloc_consolidate` |
| 399.4 | 410.1 | 10.7 | `_int_free_merge_chunk` |
| 316.6 | 323.8 | 7.2 | `_int_free_create_chunk` |
| 3,842.2 | 3,836.0 | -6.2 | `__GI_memcpy` |
| 0.1 | 5.3 | 5.2 | `_int_memalign` |
| 156.9 | 161.2 | 4.3 | `__aarch64_swp8_acq` |
| 4.0 | 0.0 | -4.0 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 127.4 | 128.8 | 1.4 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 476.1 | 477.4 | 1.3 | `__aarch64_swp4_rel` |
| 602.7 | 604.0 | 1.3 | `__aarch64_cas4_acq` |
| 1,957.4 | 1,956.1 | -1.3 | `realloc` |
| 233.6 | 234.6 | 1.0 | `unlink_chunk.isra.0` |
| 921.6 | 922.5 | 0.9 | `_int_realloc` |
| 0.8 | 0.4 | -0.5 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |
| 101.6 | 101.2 | -0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 136.9 | 137.3 | 0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 4,806.9 | 4,807.3 | 0.4 | `free` |
| 96.5 | 96.8 | 0.4 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 197.5 | 197.9 | 0.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 83.1 | 82.8 | -0.3 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 698.7 | 698.4 | -0.3 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 1.8 | 1.5 | -0.3 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 29.8 | 30.1 | 0.3 | `__aarch64_cas8_rel` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,286.1 | 2,286.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,685.2 | 1,685.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,632.0 | 1,632.0 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,490.5 | 1,490.5 | -0.1 | `library/core/src/ptr/mod.rs:847` |
| 1,477.9 | 1,479.3 | 1.3 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,083.0 | 1,083.0 | 0.0 | `library/core/src/result.rs:2177` |
| 999.0 | 999.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 739.0 | 739.6 | 0.7 | `./malloc/./malloc/malloc.c:4495` |
| 733.7 | 734.0 | 0.3 | `./malloc/./malloc/malloc.c:4527` |
| 726.0 | 726.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 666.0 | 666.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 666.0 | 666.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 666.0 | 666.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 658.0 | 658.0 | -0.0 | `library/core/src/result.rs:0` |
| 636.4 | 636.4 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 612.0 | 612.2 | 0.2 | `./malloc/./malloc/malloc.c:4658` |
| 610.4 | 610.7 | 0.4 | `./malloc/./malloc/malloc.c:4547` |
| 570.0 | 570.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 570.0 | 570.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 570.0 | 570.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 570.0 | 570.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 565.8 | 567.8 | 2.0 | `???:0` |
| 555.0 | 555.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 536.9 | 536.8 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 511.8 | 511.4 | -0.4 | `./malloc/./malloc/arena.c:147` |
| 492.7 | 493.1 | 0.4 | `./malloc/./malloc/malloc.c:4505` |
| 492.7 | 493.1 | 0.4 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 456.0 | 456.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
| 456.0 | 456.0 | -0.0 | `./malloc/./malloc/malloc.c:3371` |
| 448.0 | 448.0 | -0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 445.1 | 445.4 | 0.3 | `./malloc/./malloc/malloc.c:3163` |
| 444.0 | 444.0 | -0.0 | `./malloc/./malloc/malloc.c:3301` |
| 444.0 | 444.0 | -0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | -0.0 | `library/core/src/cmp.rs:1051` |
| 408.0 | 408.0 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 396.6 | 396.6 | -0.1 | `library/core/src/num/uint_macros.rs:2652` |
| 393.7 | 394.1 | 0.4 | `./malloc/./malloc/arena.c:153` |
| 382.3 | 382.0 | -0.3 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 346.9 | 347.0 | 0.1 | `./malloc/./malloc/malloc.c:1461` |
| 342.1 | 342.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 342.0 | 342.0 | -0.0 | `./malloc/./malloc/malloc.c:3395` |
| 342.0 | 342.0 | -0.0 | `./malloc/./malloc/malloc.c:3398` |
| 337.1 | 337.4 | 0.4 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 292.0 | 291.9 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.8 | 291.7 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 282.0 | 282.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 276.0 | 276.0 | 0.0 | `library/core/src/result.rs:2178` |
| 265.1 | 265.1 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
