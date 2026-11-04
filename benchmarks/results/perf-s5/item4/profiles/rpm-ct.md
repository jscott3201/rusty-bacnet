# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,015.8 | 7,951.0 | -64.9 | 123.01 | `_int_free` |
| 5,993.0 | 5,945.0 | -48.0 | 111.00 | `malloc` |
| 4,412.9 | 4,380.3 | -32.6 | 114.00 | `free` |
| 3,799.3 | 3,673.3 | -126.0 | 165.01 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,952.0 | 2,926.0 | -26.0 | 112.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,502.4 | 2,499.2 | -3.2 | 12.01 | `_int_malloc` |
| 2,122.0 | 2,122.0 | 0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,024.0 | 0.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,759.0 | 1,759.0 | 0.0 | 25.00 | `realloc` |
| 1,752.0 | 1,752.0 | 0.0 | 4.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,627.0 | 1,617.1 | -10.0 | 325.41 | `__aarch64_ldadd8_relax` |
| 1,620.8 | 1,620.8 | 0.0 | 40.03 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.4 | 1,540.4 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,230.0 | 1,227.0 | -3.0 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 997.8 | 997.8 | -0.0 | 16.66 | `<std::sys::pal::unix::time::Timespec>::now` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 919.8 | 919.8 | -0.0 | 12.99 | `_int_realloc` |
| 865.5 | 865.5 | -0.1 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 663.5 | 663.5 | 0.0 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 635.0 | 634.8 | -0.2 | 9.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 555.0 | 550.0 | -5.0 | 111.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 532.0 | 532.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 502.0 | 501.8 | -0.2 | 6.00 | `<tokio::runtime::time::Driver>::park_internal` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 468.3 | 468.5 | 0.2 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 416.2 | 416.4 | 0.3 | 9.01 | `_int_free_merge_chunk` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 409.5 | 409.5 | 0.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 384.5 | 384.5 | 0.0 | 12.01 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 7.00 | `<tokio::runtime::io::driver::Driver>::turn` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 373.3 | 373.5 | 0.1 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 364.3 | 0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 338.2 | 338.1 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 274.0 | 0.0 | -274.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 253.0 | 253.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_ge` |
| 0.0 | 135.0 | 135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 135.0 | 0.0 | -135.0 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 3,799.3 | 3,673.3 | -126.0 | `__GI_memcpy` |
| 95.0 | 0.0 | -95.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 0.0 | 91.0 | 91.0 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_ge` |
| 0.0 | 82.9 | 82.9 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bi` |
| 82.9 | 0.0 | -82.9 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::B` |
| 8,015.8 | 7,951.0 | -64.9 | `_int_free` |
| 0.0 | 57.0 | 57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_t` |
| 57.0 | 0.0 | -57.0 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_` |
| 5,993.0 | 5,945.0 | -48.0 | `malloc` |
| 44.0 | 0.0 | -44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::ser` |
| 0.0 | 44.0 | 44.0 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::b` |
| 41.0 | 0.0 | -41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server` |
| 0.0 | 41.0 | 41.0 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpo` |
| 4,412.9 | 4,380.3 | -32.6 | `free` |
| 2,952.0 | 2,926.0 | -26.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 22.0 | 22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bac` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<ba` |
| 0.0 | 14.0 | 14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_trans` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_general::{closure#0}>` |
| 14.0 | 0.0 | -14.0 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_serv` |
| 14.0 | 0.0 | -14.0 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>` |
| 189.0 | 202.0 | 13.0 | `bacnet_server::server::request_peer::canonical_requester` |
| 13.0 | 0.0 | -13.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BACnetServer` |
| 0.0 | 11.0 | 11.0 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>` |
| 1,627.0 | 1,617.1 | -10.0 | `__aarch64_ldadd8_relax` |
| 293.0 | 284.0 | -9.0 | `<bacnet_server::server::request_admission::Admission>::try_enter` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,297.1 | 2,297.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,675.2 | 1,664.2 | -11.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,619.8 | 1,611.9 | -7.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,595.1 | 1,590.1 | -5.0 | `library/core/src/ptr/mod.rs:847` |
| 1,476.1 | 1,464.1 | -12.0 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,078.0 | 1,078.0 | 0.0 | `library/core/src/result.rs:2177` |
| 999.0 | 990.0 | -9.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 890.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 738.0 | 732.0 | -6.0 | `./malloc/./malloc/malloc.c:4495` |
| 730.5 | 724.6 | -5.9 | `./malloc/./malloc/malloc.c:4527` |
| 728.0 | 724.0 | -4.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 705.0 | 705.0 | 0.0 | `library/core/src/result.rs:0` |
| 666.0 | 660.0 | -6.0 | `./malloc/./malloc/malloc.c:3294` |
| 666.0 | 660.0 | -6.0 | `./malloc/./malloc/malloc.c:3313` |
| 666.0 | 660.0 | -6.0 | `./malloc/./malloc/malloc.c:3354` |
| 626.1 | 622.1 | -4.0 | `./malloc/./malloc/malloc.c:1336` |
| 607.5 | 602.6 | -4.9 | `./malloc/./malloc/malloc.c:4547` |
| 597.0 | 592.0 | -5.0 | `./malloc/./malloc/malloc.c:4658` |
| 570.0 | 565.0 | -5.0 | `./malloc/./malloc/malloc.c:3392` |
| 570.0 | 565.0 | -5.0 | `./malloc/./malloc/malloc.c:3402` |
| 570.0 | 565.0 | -5.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 570.0 | 565.0 | -5.0 | `./malloc/./malloc/malloc.c:3359` |
| 566.0 | 562.0 | -4.0 | `./malloc/./malloc/arena.c:153` |
| 555.0 | 550.0 | -5.0 | `./malloc/./malloc/malloc.c:3311` |
| 492.0 | 488.0 | -4.0 | `./malloc/./malloc/malloc.c:4510` |
| 492.0 | 488.0 | -4.0 | `./malloc/./malloc/malloc.c:4505` |
| 487.6 | 487.7 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 456.0 | 452.0 | -4.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 452.0 | -4.0 | `./malloc/./malloc/malloc.c:3368` |
| 449.0 | 445.0 | -4.0 | `./malloc/./malloc/malloc.c:3163` |
| 448.0 | 444.0 | -4.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 444.0 | 440.0 | -4.0 | `./malloc/./malloc/malloc.c:3301` |
| 444.0 | 440.0 | -4.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 410.5 | 410.5 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 405.0 | 403.0 | -2.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 357.0 | 357.0 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 349.0 | 346.0 | -3.0 | `./malloc/./malloc/malloc.c:1461` |
| 345.1 | 345.1 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 342.0 | 339.0 | -3.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.0 | 339.0 | -3.0 | `./malloc/./malloc/malloc.c:3395` |
| 340.0 | 337.0 | -3.0 | `./malloc/./malloc/malloc.c:3186` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 289.3 | 289.3 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 283.0 | 283.0 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 279.0 | 279.0 | 0.0 | `library/core/src/result.rs:2178` |
| 268.9 | 268.9 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 267.1 | 267.2 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 262.0 | 262.0 | 0.0 | `library/core/src/num/uint_macros.rs:3212` |
| 262.0 | 262.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
