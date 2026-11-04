# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,305.7 | 8,171.8 | -133.9 | 125.32 | `_int_free` |
| 6,107.3 | 5,999.4 | -107.9 | 113.00 | `malloc` |
| 4,889.1 | 4,803.6 | -85.5 | 116.00 | `free` |
| 3,888.2 | 3,845.2 | -43.0 | 167.87 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | -0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,947.0 | -52.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,592.1 | 2,531.9 | -60.3 | 13.04 | `_int_malloc` |
| 2,122.0 | 2,121.9 | -0.1 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.1 | 2,022.0 | -0.1 | 30.97 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,956.5 | 1,956.8 | 0.3 | 33.06 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.1 | 1,627.1 | -20.0 | 329.43 | `__aarch64_ldadd8_relax` |
| 1,621.0 | 1,621.0 | 0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.6 | 1,540.6 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.5 | 1,234.4 | -0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | -0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 917.9 | 919.2 | 1.3 | 13.24 | `_int_realloc` |
| 865.7 | 865.7 | -0.0 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 846.9 | 846.8 | -0.1 | 11.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 698.5 | 698.6 | 0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | -0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 604.3 | 602.2 | -2.1 | 112.89 | `__aarch64_cas4_acq` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.5 | 560.5 | -10.0 | 114.10 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 532.0 | -2.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 477.7 | 475.5 | -2.2 | 90.79 | `__aarch64_swp4_rel` |
| 468.9 | 468.8 | -0.1 | 10.00 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.5 | 446.7 | 6.2 | 2.69 | `malloc_consolidate` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 425.4 | 425.2 | -0.2 | 1.35 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.0 | 397.9 | -19.1 | 9.71 | `_int_free_merge_chunk` |
| 413.0 | 545.0 | 132.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.5 | 398.4 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.1 | 379.3 | 0.2 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 379.0 | 378.7 | -0.3 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 357.5 | 357.5 | 0.0 | 11.17 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 338.4 | 338.4 | 0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 192.0 | 0.0 | -192.0 | `bacnet_transport::bvll::encode_bvll` |
| 0.0 | 180.0 | 180.0 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 8,305.7 | 8,171.8 | -133.9 | `_int_free` |
| 413.0 | 545.0 | 132.0 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 6,107.3 | 5,999.4 | -107.9 | `malloc` |
| 202.0 | 98.0 | -104.0 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 4,889.1 | 4,803.6 | -85.5 | `free` |
| 74.0 | 0.0 | -74.0 | `<bytes::bytes_mut::BytesMut as bacnet_bip_codec::sink::BvllSink>::write` |
| 70.0 | 0.0 | -70.0 | `sendto` |
| 2,592.1 | 2,531.9 | -60.3 | `_int_malloc` |
| 0.0 | 54.0 | 54.0 | `sendmsg` |
| 2,999.0 | 2,947.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 127.5 | 82.1 | -45.4 | `_mid_memalign.isra.0` |
| 3,888.2 | 3,845.2 | -43.0 | `__GI_memcpy` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<<bacnet_transport::bip::node::NodeHandle>::send_unicast::{closure#0}>` |
| 332.4 | 310.5 | -21.9 | `_int_free_create_chunk` |
| 1,647.1 | 1,627.1 | -20.0 | `__aarch64_ldadd8_relax` |
| 417.0 | 397.9 | -19.1 | `_int_free_merge_chunk` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu_with_data_attributes::{closure#0}>` |
| 11.0 | 0.0 | -11.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}>` |
| 10.5 | 0.0 | -10.5 | `_int_memalign` |
| 570.5 | 560.5 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 221.2 | 214.8 | -6.4 | `unlink_chunk.isra.0` |
| 440.5 | 446.7 | 6.2 | `malloc_consolidate` |
| 477.7 | 475.5 | -2.2 | `__aarch64_swp4_rel` |
| 604.3 | 602.2 | -2.1 | `__aarch64_cas4_acq` |
| 534.0 | 532.0 | -2.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 76.0 | 78.0 | 2.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}` |
| 17.7 | 19.1 | 1.4 | `alloc_perturb` |
| 917.9 | 919.2 | 1.3 | `_int_realloc` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,321.1 | 2,287.1 | -34.0 | `library/core/src/ptr/mod.rs:573` |
| 1,707.3 | 1,685.2 | -22.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,648.0 | 1,632.0 | -16.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,503.8 | 1,477.9 | -25.8 | `./malloc/./malloc/malloc.c:4518` |
| 1,497.5 | 1,490.5 | -7.0 | `library/core/src/ptr/mod.rs:847` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,095.0 | 1,083.0 | -12.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 999.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 751.9 | 739.0 | -12.9 | `./malloc/./malloc/malloc.c:4495` |
| 745.6 | 733.7 | -11.9 | `./malloc/./malloc/malloc.c:4527` |
| 734.0 | 726.0 | -8.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 678.0 | 666.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 666.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 666.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 664.0 | 658.0 | -6.0 | `library/core/src/result.rs:0` |
| 645.9 | 636.3 | -9.6 | `./malloc/./malloc/malloc.c:1336` |
| 622.3 | 611.9 | -10.4 | `./malloc/./malloc/malloc.c:4658` |
| 620.4 | 610.5 | -9.8 | `./malloc/./malloc/malloc.c:4547` |
| 580.1 | 570.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.1 | 570.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.1 | 570.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 570.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 568.4 | 568.4 | -0.0 | `???:0` |
| 565.0 | 555.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 536.9 | 536.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 530.1 | 505.5 | -24.6 | `./malloc/./malloc/arena.c:147` |
| 501.3 | 492.7 | -8.6 | `./malloc/./malloc/malloc.c:4505` |
| 501.3 | 492.7 | -8.6 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 456.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 456.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 448.0 | -8.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.6 | 444.1 | -8.5 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 444.0 | -8.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 444.0 | -8.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 412.0 | 408.0 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 396.8 | 395.8 | -1.0 | `./malloc/./malloc/arena.c:153` |
| 396.6 | 396.6 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 384.3 | 382.4 | -1.9 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.6 | 346.2 | -6.4 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 342.0 | -6.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 342.0 | -6.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.6 | 336.3 | -6.3 | `./malloc/./malloc/malloc.c:3186` |
| 342.3 | 342.1 | -0.2 | `library/core/src/ptr/mod.rs:1963` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 292.0 | 292.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.7 | 291.8 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 280.0 | 276.0 | -4.0 | `library/core/src/result.rs:2178` |
| 270.0 | 262.0 | -8.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 267.1 | 265.1 | -2.0 | `library/core/src/ptr/mut_ptr.rs:970` |
