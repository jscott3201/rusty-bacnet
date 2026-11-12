# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,181.4 | 8,178.5 | -2.9 | 123.27 | `_int_free` |
| 5,999.5 | 5,999.4 | -0.1 | 111.00 | `malloc` |
| 4,801.2 | 4,800.0 | -1.1 | 114.00 | `free` |
| 3,902.5 | 3,909.1 | 6.6 | 165.89 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,119.0 | 3,119.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,947.0 | 2,947.0 | 0.0 | 112.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,604.7 | 2,566.3 | -38.4 | 13.21 | `_int_malloc` |
| 2,121.9 | 2,121.9 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,957.4 | 1,954.1 | -3.3 | 32.76 | `realloc` |
| 1,752.0 | 1,752.0 | 0.0 | 4.05 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,627.0 | 1,609.6 | -17.4 | 325.41 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | -0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,615.0 | 1,615.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,244.6 | 1,244.6 | -0.0 | 49.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 950.0 | 950.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 917.7 | 919.9 | 2.3 | 13.22 | `_int_realloc` |
| 865.6 | 879.1 | 13.5 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 820.0 | 820.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 732.0 | 732.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 724.7 | 725.3 | 0.6 | 9.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 698.4 | 698.4 | 0.1 | 8.22 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | -0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 604.5 | 602.9 | -1.6 | 112.91 | `__aarch64_cas4_acq` |
| 560.7 | 559.7 | -1.0 | 112.14 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 477.9 | 476.3 | -1.6 | 90.81 | `__aarch64_swp4_rel` |
| 468.3 | 297.2 | -171.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 441.7 | 439.0 | -2.7 | 2.72 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 425.6 | 425.3 | -0.3 | 1.34 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 415.1 | 401.5 | -13.6 | 9.64 | `_int_free_merge_chunk` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.6 | 224.0 | -174.6 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.1 | 292.0 | -87.0 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.9 | 378.7 | -0.2 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 352.0 | 352.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 338.3 | 338.3 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 398.6 | 224.0 | -174.6 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.3 | 297.2 | -171.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.0 | 0.0 | -117.0 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 87.3 | 0.0 | -87.3 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 379.1 | 292.0 | -87.0 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 76.5 | 0.1 | -76.4 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.0 | 72.0 | 72.0 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.0 | 63.0 | 63.0 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closure#0}::{cl` |
| 107.9 | 59.1 | -48.9 | `tokio::runtime::task::waker::drop_waker` |
| 164.1 | 115.3 | -48.8 | `tokio::runtime::task::waker::clone_waker` |
| 2,604.7 | 2,566.3 | -38.4 | `_int_malloc` |
| 93.1 | 62.0 | -31.1 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 230.5 | 213.0 | -17.5 | `__aarch64_cas8_acq_rel` |
| 59.9 | 42.5 | -17.5 | `__aarch64_cas8_acq` |
| 113.4 | 96.0 | -17.4 | `__aarch64_ldadd8_acq_rel` |
| 1,627.0 | 1,609.6 | -17.4 | `__aarch64_ldadd8_relax` |
| 415.1 | 401.5 | -13.6 | `_int_free_merge_chunk` |
| 865.6 | 879.1 | 13.5 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 327.0 | 315.6 | -11.4 | `_int_free_create_chunk` |
| 82.2 | 71.1 | -11.0 | `_int_free_maybe_consolidate.part.0` |
| 3,902.5 | 3,909.1 | 6.6 | `__GI_memcpy` |
| 163.1 | 157.6 | -5.5 | `__aarch64_swp8_acq` |
| 218.0 | 213.2 | -4.8 | `unlink_chunk.isra.0` |
| 96.7 | 100.3 | 3.7 | `_mid_memalign.isra.0` |
| 1,957.4 | 1,954.1 | -3.3 | `realloc` |
| 8,181.4 | 8,178.5 | -2.9 | `_int_free` |
| 441.7 | 439.0 | -2.7 | `malloc_consolidate` |
| 917.7 | 919.9 | 2.3 | `_int_realloc` |
| 604.5 | 602.9 | -1.6 | `__aarch64_cas4_acq` |
| 477.9 | 476.3 | -1.6 | `__aarch64_swp4_rel` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,266.1 | 2,266.1 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,685.2 | 1,674.8 | -10.5 | `library/core/src/sync/atomic.rs:4029` |
| 1,631.8 | 1,604.0 | -27.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,490.5 | 1,492.0 | 1.5 | `library/core/src/ptr/mod.rs:847` |
| 1,479.3 | 1,477.6 | -1.6 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,083.0 | 1,083.0 | 0.0 | `library/core/src/result.rs:2177` |
| 999.0 | 999.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 739.6 | 738.8 | -0.8 | `./malloc/./malloc/malloc.c:4495` |
| 733.8 | 733.7 | -0.2 | `./malloc/./malloc/malloc.c:4527` |
| 726.0 | 726.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 666.0 | 666.0 | -0.0 | `library/core/src/result.rs:0` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 639.8 | 637.3 | -2.5 | `./malloc/./malloc/malloc.c:1336` |
| 612.2 | 611.9 | -0.3 | `./malloc/./malloc/malloc.c:4658` |
| 610.6 | 610.5 | -0.0 | `./malloc/./malloc/malloc.c:4547` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 570.0 | 570.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 569.3 | 562.4 | -6.9 | `???:0` |
| 555.0 | 555.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 536.7 | 508.8 | -27.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 501.4 | 497.8 | -3.6 | `./malloc/./malloc/arena.c:147` |
| 493.1 | 492.5 | -0.5 | `./malloc/./malloc/malloc.c:4505` |
| 493.1 | 492.5 | -0.5 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 456.0 | 456.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 456.0 | 456.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 448.0 | 448.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 444.8 | 444.4 | -0.5 | `./malloc/./malloc/malloc.c:3163` |
| 444.0 | 444.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 444.0 | 444.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 408.0 | 401.0 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 399.5 | 399.3 | -0.1 | `./malloc/./malloc/arena.c:153` |
| 396.5 | 394.5 | -2.0 | `library/core/src/num/uint_macros.rs:2652` |
| 382.1 | 382.2 | 0.1 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 346.7 | 346.5 | -0.3 | `./malloc/./malloc/malloc.c:1461` |
| 342.1 | 342.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 342.0 | 342.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 342.0 | 342.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 337.1 | 336.5 | -0.6 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 291.9 | 292.0 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.7 | 291.8 | 0.1 | `library/core/src/sync/atomic.rs:4083` |
| 282.0 | 282.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 278.0 | 278.0 | 0.0 | `library/core/src/result.rs:2178` |
| 275.1 | 275.1 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
