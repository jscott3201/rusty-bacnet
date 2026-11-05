# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,052.8 | 8,052.8 | 0.0 | 123.67 | `_int_free` |
| 5,993.0 | 5,993.0 | 0.0 | 111.00 | `malloc` |
| 4,400.1 | 4,400.1 | -0.0 | 114.00 | `free` |
| 3,842.5 | 3,842.5 | -0.0 | 166.00 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,120.0 | 3,120.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,990.1 | 2,990.1 | -0.1 | 13.00 | `_int_malloc` |
| 2,952.0 | 2,952.0 | 0.0 | 112.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,752.0 | 1,752.0 | 0.0 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,627.0 | 1,627.0 | -0.0 | 325.41 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | 0.0 | 40.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,585.0 | 1,585.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 950.0 | 950.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 927.4 | 805.5 | -122.0 | 14.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 896.2 | 896.2 | 0.0 | 13.00 | `_int_realloc` |
| 865.5 | 865.4 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 820.0 | 820.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 711.0 | 711.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 698.1 | 698.1 | -0.0 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 555.0 | 555.0 | 0.0 | 111.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 532.0 | 736.0 | 204.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 500.4 | 500.4 | 0.1 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.2 | 468.4 | 0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 429.8 | 429.8 | -0.0 | 9.34 | `_int_free_merge_chunk` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.5 | 398.5 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 391.1 | 391.1 | -0.0 | 18.66 | `unlink_chunk.isra.0` |
| 384.5 | 320.5 | -64.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 384.0 | 384.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 378.0 | 378.0 | -0.0 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 354.8 | 354.8 | -0.0 | 9.34 | `_int_free_create_chunk` |
| 338.1 | 338.1 | 0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 234.0 | 0.0 | -234.0 | `<bacnet_time::host::HostClock>::now` |
| 532.0 | 736.0 | 204.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 927.4 | 805.5 | -122.0 | `<std::sys::pal::unix::time::Timespec>::now` |
| 384.5 | 320.5 | -64.0 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 150.1 | 130.1 | -20.0 | `clock_gettime@@GLIBC_2.17` |
| 4.0 | 0.0 | -4.0 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 137.3 | 136.8 | -0.5 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 101.3 | 100.9 | -0.4 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 83.1 | 82.7 | -0.4 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 117.4 | 117.7 | 0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.7 | 87.9 | 0.2 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 93.0 | 93.2 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 468.2 | 468.4 | 0.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 96.7 | 96.6 | -0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 251.0 | 250.9 | -0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 398.5 | 398.5 | 0.1 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 865.5 | 865.4 | -0.1 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 2,990.1 | 2,990.1 | -0.1 | `_int_malloc` |
| 76.6 | 76.6 | 0.1 | `<tokio::time::interval::Interval>::poll_tick` |
| 500.4 | 500.4 | 0.1 | `<tokio::runtime::time::Driver>::park_internal` |
| 298.9 | 298.8 | -0.1 | `__aarch64_swp4_rel` |
| 328.9 | 328.8 | -0.1 | `__aarch64_cas4_acq` |
| 51.3 | 51.4 | 0.0 | `tokio::util::wake::clone_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 186.4 | 186.5 | 0.0 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 44.0 | 44.0 | 0.0 | `tokio::util::wake::drop_arc_raw::<tokio::runtime::scheduler::current_thread::Handle>` |
| 378.0 | 378.0 | -0.0 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 121.7 | 121.7 | 0.0 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 698.1 | 698.1 | -0.0 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 59.9 | 59.9 | 0.0 | `__aarch64_cas8_acq` |
| 195.4 | 195.4 | 0.0 | `__aarch64_cas8_acq_rel` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,286.0 | 2,286.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,682.2 | 1,682.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,603.8 | 1,603.8 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,507.0 | 1,507.0 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,484.0 | 1,484.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,083.0 | 1,083.0 | 0.0 | `library/core/src/result.rs:2177` |
| 999.0 | 999.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 742.0 | 742.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 737.0 | 737.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 726.0 | 726.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 658.0 | 658.0 | 0.0 | `library/core/src/result.rs:0` |
| 636.0 | 636.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 613.3 | 613.3 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 599.7 | 599.7 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 570.0 | 570.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 566.0 | 566.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 555.0 | 555.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 494.7 | 494.7 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 494.7 | 494.7 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 471.6 | 471.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 456.0 | 456.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 456.0 | 456.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 448.0 | 448.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 445.0 | 445.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 444.0 | 444.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 444.0 | 444.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 410.5 | 410.5 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 401.0 | 400.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 358.0 | 357.9 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 346.0 | 346.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 342.0 | 342.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.0 | 342.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 335.0 | 335.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 282.0 | 282.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 276.0 | 276.0 | 0.0 | `library/core/src/result.rs:2178` |
| 271.0 | 271.0 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 267.1 | 267.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.9 | 265.8 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 262.0 | 254.0 | -8.0 | `library/core/src/num/uint_macros.rs:3212` |
| 260.0 | 260.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:655` |
