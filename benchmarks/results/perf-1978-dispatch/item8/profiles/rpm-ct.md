# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,029.8 | 8,124.5 | 94.8 | 123.50 | `_int_free` |
| 5,993.0 | 5,993.0 | 0.0 | 111.00 | `malloc` |
| 4,407.7 | 4,397.1 | -10.6 | 114.00 | `free` |
| 3,904.0 | 3,901.2 | -2.9 | 165.77 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,119.0 | 3,119.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,952.0 | 2,952.0 | 0.0 | 112.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,614.1 | 2,908.3 | 294.3 | 12.77 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,022.0 | 2,022.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,754.1 | 1,738.0 | -16.1 | 25.00 | `realloc` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,627.0 | 1,609.5 | -17.5 | 325.40 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | 0.0 | 40.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,615.0 | 1,615.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,240.0 | 1,240.0 | 0.0 | 49.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 950.0 | 950.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 896.1 | 927.8 | 31.7 | 13.23 | `_int_realloc` |
| 865.4 | 879.0 | 13.6 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 820.0 | 820.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 805.5 | 805.2 | -0.3 | 12.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 732.0 | 732.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 698.1 | 698.1 | 0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 555.0 | 555.0 | 0.0 | 111.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 501.1 | 499.3 | -1.8 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.1 | 296.6 | -171.5 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 436.8 | 497.0 | 60.2 | 9.50 | `_int_free_merge_chunk` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 398.4 | 224.0 | -174.4 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 378.2 | 290.4 | -87.8 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 352.0 | 352.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 346.4 | 418.0 | 71.6 | 9.50 | `_int_free_create_chunk` |
| 338.1 | 338.1 | 0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 328.9 | 328.8 | -0.1 | 65.77 | `__aarch64_cas4_acq` |
| 320.4 | 320.4 | 0.0 | 10.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 2,614.1 | 2,908.3 | 294.3 | `_int_malloc` |
| 398.4 | 224.0 | -174.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.1 | 296.6 | -171.5 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.3 | 0.0 | -117.3 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 8,029.8 | 8,124.5 | 94.8 | `_int_free` |
| 378.2 | 290.4 | -87.8 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 87.6 | 0.0 | -87.6 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 76.6 | 0.1 | -76.5 | `<tokio::time::interval::Interval>::poll_tick` |
| 309.6 | 382.8 | 73.3 | `unlink_chunk.isra.0` |
| 0.0 | 72.0 | 72.0 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 346.4 | 418.0 | 71.6 | `_int_free_create_chunk` |
| 0.0 | 63.0 | 63.0 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closure#0}::{cl` |
| 436.8 | 497.0 | 60.2 | `_int_free_merge_chunk` |
| 107.9 | 58.9 | -49.0 | `tokio::runtime::task::waker::drop_waker` |
| 164.0 | 115.1 | -49.0 | `tokio::runtime::task::waker::clone_waker` |
| 896.1 | 927.8 | 31.7 | `_int_realloc` |
| 92.7 | 62.0 | -30.7 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 255.5 | 225.0 | -30.5 | `malloc_consolidate` |
| 1,627.0 | 1,609.5 | -17.5 | `__aarch64_ldadd8_relax` |
| 195.4 | 177.9 | -17.5 | `__aarch64_cas8_acq_rel` |
| 59.9 | 42.4 | -17.5 | `__aarch64_cas8_acq` |
| 83.6 | 66.1 | -17.5 | `__aarch64_ldadd8_acq_rel` |
| 1,754.1 | 1,738.0 | -16.1 | `realloc` |
| 106.1 | 90.0 | -16.1 | `__aarch64_swp8_acq` |
| 865.4 | 879.0 | 13.6 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 4,407.7 | 4,397.1 | -10.6 | `free` |
| 23.7 | 28.0 | 4.3 | `_int_free_maybe_consolidate.part.0` |
| 10.7 | 14.0 | 3.3 | `alloc_perturb` |
| 3,904.0 | 3,901.2 | -2.9 | `__GI_memcpy` |
| 185.2 | 187.9 | 2.6 | `<tokio::runtime::time::handle::Handle>::process_at_time` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,266.0 | 2,266.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,682.2 | 1,671.7 | -10.5 | `library/core/src/sync/atomic.rs:4029` |
| 1,603.8 | 1,575.7 | -28.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,507.0 | 1,508.5 | 1.5 | `library/core/src/ptr/mod.rs:847` |
| 1,482.0 | 1,500.0 | 18.0 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,083.0 | 1,083.0 | 0.0 | `library/core/src/result.rs:2177` |
| 999.0 | 999.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 888.0 | 888.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 741.0 | 750.0 | 9.0 | `./malloc/./malloc/malloc.c:4495` |
| 734.7 | 745.0 | 10.3 | `./malloc/./malloc/malloc.c:4527` |
| 726.0 | 726.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 666.0 | 666.0 | 0.0 | `library/core/src/result.rs:0` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 666.0 | 666.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 633.7 | 636.0 | 2.3 | `./malloc/./malloc/malloc.c:1336` |
| 611.2 | 620.0 | 8.8 | `./malloc/./malloc/malloc.c:4547` |
| 598.5 | 603.0 | 4.5 | `./malloc/./malloc/malloc.c:4658` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 570.0 | 570.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 570.0 | 570.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 566.0 | 566.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 555.0 | 555.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 494.0 | 500.0 | 6.0 | `./malloc/./malloc/malloc.c:4510` |
| 494.0 | 500.0 | 6.0 | `./malloc/./malloc/malloc.c:4505` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 471.6 | 443.5 | -28.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 456.0 | 456.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 448.0 | 448.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 446.2 | 445.0 | -1.2 | `./malloc/./malloc/malloc.c:3163` |
| 444.0 | 444.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 444.0 | 444.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 410.5 | 408.5 | -2.0 | `library/core/src/num/uint_macros.rs:2652` |
| 400.9 | 393.9 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 358.0 | 357.9 | -0.1 | `library/core/src/sync/atomic.rs:3994` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 346.7 | 346.0 | -0.7 | `./malloc/./malloc/malloc.c:1461` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 342.0 | 342.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 342.0 | 342.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 336.2 | 335.0 | -1.2 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 282.0 | 282.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 281.0 | 281.0 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 278.0 | 278.0 | 0.0 | `library/core/src/result.rs:2178` |
| 267.1 | 267.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 265.9 | 265.8 | -0.1 | `library/core/src/sync/atomic.rs:4083` |
| 260.0 | 260.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:655` |
| 254.0 | 254.0 | 0.0 | `library/core/src/num/uint_macros.rs:3212` |
