# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,182.8 | 8,052.8 | -130.0 | 125.67 | `_int_free` |
| 6,101.0 | 5,993.0 | -108.0 | 113.00 | `malloc` |
| 4,470.1 | 4,402.1 | -68.0 | 116.00 | `free` |
| 3,890.5 | 3,842.5 | -48.0 | 168.00 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 3,004.0 | 2,952.0 | -52.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,990.1 | 2,990.1 | 0.0 | 13.00 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,752.0 | 1,752.0 | 0.0 | 25.00 | `realloc` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.0 | 1,626.9 | -20.1 | 329.41 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | -0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,230.0 | 0.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 927.7 | 927.4 | -0.2 | 14.65 | `<std::sys::pal::unix::time::Timespec>::now` |
| 896.2 | 896.2 | 0.0 | 13.00 | `_int_realloc` |
| 865.4 | 865.6 | 0.2 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 698.1 | 697.9 | -0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 565.0 | 555.0 | -10.0 | 113.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 532.0 | -2.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 501.6 | 500.5 | -1.1 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.1 | 467.5 | -0.6 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 429.8 | 429.8 | -0.0 | 9.34 | `_int_free_merge_chunk` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 545.0 | 132.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.6 | 398.5 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 391.1 | 391.1 | 0.0 | 18.65 | `unlink_chunk.isra.0` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.5 | 384.5 | -0.0 | 12.02 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 378.0 | 377.4 | -0.6 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 354.8 | 354.8 | 0.0 | 9.34 | `_int_free_create_chunk` |
| 338.1 | 338.1 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 192.0 | 0.0 | -192.0 | `bacnet_transport::bvll::encode_bvll` |
| 0.0 | 180.0 | 180.0 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 413.0 | 545.0 | 132.0 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 8,182.8 | 8,052.8 | -130.0 | `_int_free` |
| 6,101.0 | 5,993.0 | -108.0 | `malloc` |
| 202.0 | 98.0 | -104.0 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 74.0 | 0.0 | -74.0 | `<bytes::bytes_mut::BytesMut as bacnet_bip_codec::sink::BvllSink>::write` |
| 4,470.1 | 4,402.1 | -68.0 | `free` |
| 3,004.0 | 2,952.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 3,890.5 | 3,842.5 | -48.0 | `__GI_memcpy` |
| 46.0 | 0.0 | -46.0 | `sendto` |
| 0.0 | 38.0 | 38.0 | `sendmsg` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<<bacnet_transport::bip::node::NodeHandle>::send_unicast::{closure#0}>` |
| 1,647.0 | 1,626.9 | -20.1 | `__aarch64_ldadd8_relax` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu_with_data_attributes::{closure#0}>` |
| 11.0 | 0.0 | -11.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}>` |
| 565.0 | 555.0 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 534.0 | 532.0 | -2.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 76.0 | 78.0 | 2.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}` |
| 184.6 | 186.2 | 1.6 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 501.6 | 500.5 | -1.1 | `<tokio::runtime::time::Driver>::park_internal` |
| 57.0 | 56.0 | -1.0 | `<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu::{closure#0}` |
| 378.0 | 377.4 | -0.6 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 468.1 | 467.5 | -0.6 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 101.4 | 100.9 | -0.5 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 117.3 | 117.5 | 0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 96.3 | 96.5 | 0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 927.7 | 927.4 | -0.2 | `<std::sys::pal::unix::time::Timespec>::now` |
| 83.0 | 82.8 | -0.2 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 108.0 | 107.8 | -0.2 | `tokio::runtime::task::waker::drop_waker` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,321.1 | 2,287.1 | -34.0 | `library/core/src/ptr/mod.rs:573` |
| 1,704.2 | 1,682.1 | -22.1 | `library/core/src/sync/atomic.rs:4029` |
| 1,619.8 | 1,603.7 | -16.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,514.1 | 1,507.0 | -7.0 | `library/core/src/ptr/mod.rs:847` |
| 1,508.0 | 1,484.0 | -24.0 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,095.0 | 1,083.0 | -12.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 999.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 754.0 | 742.0 | -12.0 | `./malloc/./malloc/malloc.c:4495` |
| 749.0 | 737.0 | -12.0 | `./malloc/./malloc/malloc.c:4527` |
| 734.0 | 726.0 | -8.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 678.0 | 666.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 666.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 666.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 664.0 | 658.0 | -6.0 | `library/core/src/result.rs:0` |
| 644.0 | 636.0 | -8.0 | `./malloc/./malloc/malloc.c:1336` |
| 623.3 | 613.3 | -10.0 | `./malloc/./malloc/malloc.c:4547` |
| 609.7 | 599.7 | -10.0 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 570.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 570.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 570.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 570.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 574.0 | 566.0 | -8.0 | `./malloc/./malloc/arena.c:153` |
| 565.0 | 555.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 502.7 | 494.7 | -8.0 | `./malloc/./malloc/malloc.c:4510` |
| 502.7 | 494.7 | -8.0 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 471.6 | 471.5 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 464.0 | 456.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 456.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 448.0 | -8.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 453.0 | 445.0 | -8.0 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 444.0 | -8.0 | `./malloc/./malloc/malloc.c:3316` |
| 452.0 | 444.0 | -8.0 | `./malloc/./malloc/malloc.c:3301` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 410.5 | 410.5 | -0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 405.0 | 400.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 360.0 | 357.9 | -2.0 | `library/core/src/sync/atomic.rs:3994` |
| 352.0 | 346.0 | -6.0 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 342.0 | -6.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 342.0 | -6.0 | `./malloc/./malloc/malloc.c:3398` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 341.0 | 335.0 | -6.0 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 280.0 | 276.0 | -4.0 | `library/core/src/result.rs:2178` |
| 273.0 | 271.0 | -2.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 270.0 | 262.0 | -8.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 267.1 | 267.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.9 | 265.8 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 262.0 | 262.0 | 0.0 | `library/core/src/num/uint_macros.rs:3212` |
| 260.0 | 260.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:655` |
