# rpm / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,306.2 | 8,311.6 | 5.4 | 125.16 | `_int_free` |
| 6,107.5 | 6,107.6 | 0.1 | 113.00 | `malloc` |
| 4,878.9 | 4,892.7 | 13.8 | 116.00 | `free` |
| 3,891.1 | 3,894.7 | 3.7 | 167.95 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,100.0 | 3,100.0 | 0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,999.0 | 2,999.0 | -0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,543.1 | 2,788.6 | 245.5 | 13.23 | `_int_malloc` |
| 2,122.0 | 2,122.0 | -0.0 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,024.0 | 2,022.0 | -2.0 | 31.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::read_property_multiple::{closure#0}` |
| 1,956.3 | 1,958.8 | 2.4 | 32.81 | `realloc` |
| 1,752.0 | 1,752.0 | -0.0 | 4.01 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,698.0 | 1,698.0 | 0.0 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,647.3 | 1,647.2 | -0.1 | 329.46 | `__aarch64_ldadd8_relax` |
| 1,621.5 | 1,621.0 | -0.5 | 40.05 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,575.0 | 1,575.0 | 0.0 | 25.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,540.8 | 1,540.6 | -0.3 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,234.8 | 1,234.1 | -0.6 | 48.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 1,200.0 | 1,200.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 1,186.0 | 1,186.0 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,160.0 | 1,160.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,130.0 | 1,130.0 | 0.0 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 918.8 | 917.7 | -1.1 | 13.18 | `<std::sys::pal::unix::time::Timespec>::now` |
| 914.1 | 912.3 | -1.8 | 13.19 | `_int_realloc` |
| 865.9 | 865.9 | -0.0 | 6.01 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 850.0 | 850.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 800.0 | 800.0 | 0.0 | 30.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::decode_property_reference` |
| 689.6 | 689.6 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 664.1 | 258.6 | -405.4 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 636.1 | 635.4 | -0.6 | 9.23 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 630.0 | 630.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 605.7 | 605.8 | 0.0 | 113.13 | `__aarch64_cas4_acq` |
| 601.0 | 601.0 | 0.0 | 10.00 | `<bacnet_services::rpm::ReadResultElement>::encode` |
| 570.0 | 569.8 | -0.2 | 113.99 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 534.0 | 534.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 534.0 | 534.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 490.0 | 490.0 | 0.0 | 10.00 | `<bacnet_objects::event::history::EventHistory>::read` |
| 480.0 | 480.0 | 0.0 | 1.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 479.1 | 479.2 | 0.1 | 91.03 | `__aarch64_swp4_rel` |
| 468.9 | 469.1 | 0.2 | 10.00 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 440.0 | 440.0 | 0.0 | 10.00 | `<bacnet_objects::object_profile::saving::ProfileState>::read` |
| 432.9 | 436.0 | 3.0 | 2.62 | `malloc_consolidate` |
| 430.0 | 430.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 428.0 | 428.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 425.7 | 428.4 | 2.7 | 1.38 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 413.0 | 0.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 410.2 | 81.6 | -328.6 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 405.3 | 437.2 | 31.9 | 9.41 | `_int_free_merge_chunk` |
| 385.0 | 385.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.4 | 381.5 | 2.2 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 375.5 | 379.4 | 3.9 | 9.46 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 364.2 | 398.6 | 34.4 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 664.1 | 258.6 | -405.4 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 410.2 | 81.6 | -328.6 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 286.1 | 0.0 | -286.1 | `<tokio::sync::mutex::Mutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock::{closure#0}` |
| 2,543.1 | 2,788.6 | 245.5 | `_int_malloc` |
| 14.0 | 176.0 | 162.0 | `<bytes::bytes_mut::BytesMut>::with_capacity` |
| 218.0 | 102.0 | -116.0 | `bacnet_server::server::requests::issue::response_limits` |
| 60.0 | 156.0 | 96.0 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |
| 28.0 | 107.0 | 79.0 | `<smallvec::SmallVec<[u8; 6]>>::from_slice` |
| 268.0 | 318.0 | 50.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 196.9 | 246.8 | 49.8 | `unlink_chunk.isra.0` |
| 37.0 | 86.0 | 49.0 | `bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}` |
| 92.0 | 45.0 | -47.0 | `bacnet_client::client::response_admission::take_current_reassembly::{closure#0}` |
| 317.4 | 364.4 | 46.9 | `_int_free_create_chunk` |
| 129.4 | 173.4 | 44.0 | `_mid_memalign.isra.0` |
| 50.1 | 10.1 | -40.0 | `<tokio::sync::batch_semaphore::Acquire as core::ops::drop::Drop>::drop` |
| 17.0 | 53.0 | 36.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}>` |
| 215.9 | 181.0 | -35.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 364.2 | 398.6 | 34.4 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 405.3 | 437.2 | 31.9 | `_int_free_merge_chunk` |
| 135.9 | 158.9 | 23.0 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 235.8 | 213.8 | -22.0 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 93.0 | 72.0 | -21.0 | `<bacnet_client::tsm::Tsm>::complete_transaction_inner` |
| 251.0 | 230.7 | -20.4 | `__aarch64_cas8_acq_rel` |
| 233.6 | 213.5 | -20.1 | `__aarch64_ldadd8_rel` |
| 0.0 | 20.0 | 20.0 | `<bacnet_client::sync_mutex::SyncMutex<std::collections::hash::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::client::SegmentAckRoute>>>::lock` |
| 0.0 | 17.0 | 17.0 | `core::ptr::drop_glue::<tokio::util::sharded_list::ShardGuard<tokio::runtime::task::Task<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>>>` |
| 0.0 | 16.0 | 16.0 | `<core::result::Result<(u8, bacnet_client::tsm::TransactionRegistration), bacnet_client::tsm::coordinated::CoordinatedRegistrationError>>::map_err::<bacnet_types` |
| 74.0 | 59.0 | -15.0 | `<bacnet_encoding::apdu::ComplexAck as core::clone::Clone>::clone` |
| 4,878.9 | 4,892.7 | 13.8 | `free` |
| 12.0 | 0.0 | -12.0 | `core::ptr::drop_glue::<bacnet_client::client::response_admission::admit_terminal_during_reassembly::{closure#0}>` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 2,331.1 | 2,320.1 | -11.0 | `library/core/src/ptr/mod.rs:573` |
| 1,700.3 | 1,707.3 | 7.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,664.4 | 1,648.1 | -16.3 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,585.0 | 1,535.7 | -49.3 | `library/core/src/ptr/mod.rs:847` |
| 1,501.9 | 1,504.6 | 2.7 | `./malloc/./malloc/malloc.c:4518` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,090.0 | 1,095.0 | 5.0 | `library/core/src/result.rs:2177` |
| 1,017.0 | 1,017.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 890.0 | 888.0 | -2.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 751.0 | 752.4 | 1.4 | `./malloc/./malloc/malloc.c:4495` |
| 745.8 | 746.2 | 0.3 | `./malloc/./malloc/malloc.c:4527` |
| 736.0 | 734.0 | -2.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 711.1 | 664.0 | -47.1 | `library/core/src/result.rs:0` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | -0.0 | `./malloc/./malloc/malloc.c:3294` |
| 647.2 | 651.5 | 4.3 | `./malloc/./malloc/malloc.c:1336` |
| 621.9 | 622.4 | 0.4 | `./malloc/./malloc/malloc.c:4658` |
| 620.7 | 620.9 | 0.2 | `./malloc/./malloc/malloc.c:4547` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.1 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | -0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 566.7 | 569.2 | 2.5 | `???:0` |
| 565.0 | 565.0 | -0.0 | `./malloc/./malloc/malloc.c:3311` |
| 553.7 | 537.0 | -16.6 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 513.1 | 531.6 | 18.5 | `./malloc/./malloc/arena.c:147` |
| 500.7 | 501.6 | 0.9 | `./malloc/./malloc/malloc.c:4505` |
| 500.7 | 501.6 | 0.9 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | -0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 452.8 | 451.5 | -1.4 | `./malloc/./malloc/malloc.c:3163` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | -0.0 | `./malloc/./malloc/malloc.c:3316` |
| 435.0 | 435.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 416.1 | 412.0 | -4.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 401.4 | 396.5 | -4.9 | `./malloc/./malloc/arena.c:153` |
| 396.9 | 396.7 | -0.3 | `library/core/src/num/uint_macros.rs:2652` |
| 383.5 | 384.3 | 0.7 | `library/core/src/sync/atomic.rs:3994` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 352.7 | 352.7 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 350.0 | 350.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 342.8 | 341.4 | -1.4 | `./malloc/./malloc/malloc.c:3186` |
| 342.6 | 342.7 | 0.1 | `library/core/src/ptr/mod.rs:1963` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 310.0 | 310.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 300.0 | 300.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 295.1 | 291.8 | -3.3 | `library/core/src/sync/atomic.rs:4083` |
| 294.6 | 294.1 | -0.5 | `library/std/src/sys/pal/unix/time.rs:107` |
| 292.4 | 292.1 | -0.3 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 283.0 | 280.0 | -3.0 | `library/core/src/result.rs:2178` |
| 279.2 | 271.1 | -8.0 | `library/core/src/ptr/mut_ptr.rs:970` |
