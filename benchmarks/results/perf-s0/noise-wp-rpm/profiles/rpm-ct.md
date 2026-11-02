# rpm / ct: self instructions per request

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 8,731.3 | 8,714.4 | -16.9 | 128.80 | `_int_free` |
| 6,767.2 | 6,727.5 | -39.7 | 32.21 | `_int_malloc` |
| 6,100.7 | 6,105.2 | 4.6 | 113.00 | `malloc` |
| 4,736.0 | 4,722.6 | -13.5 | 116.00 | `free` |
| 4,404.1 | 4,406.1 | 2.0 | 193.74 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 150.00 | `bacnet_encoding::tags::decode_tag` |
| 3,250.0 | 3,250.0 | -0.0 | 71.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,989.0 | 2,989.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,576.5 | 2,578.1 | 1.6 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,132.1 | 2,133.0 | 0.8 | 6.88 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,785.4 | 1,780.7 | -4.7 | 25.00 | `realloc` |
| 1,654.0 | 1,654.0 | 0.0 | 30.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,648.8 | 1,648.7 | -0.0 | 329.76 | `__aarch64_ldadd8_relax` |
| 1,621.5 | 1,621.5 | 0.0 | 40.05 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,621.0 | 1,621.0 | -0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,561.1 | 1,561.1 | 0.0 | 20.01 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,435.9 | 1,435.7 | -0.2 | 10.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,210.0 | 1,210.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,160.0 | 1,160.0 | 0.0 | 20.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 1,125.8 | 1,111.4 | -14.4 | 10.00 | `bacnet_encoding::tags::extract_context_value` |
| 990.0 | 990.0 | 0.0 | 10.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 960.0 | 960.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 894.0 | 894.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::encode_property_reference` |
| 851.5 | 885.0 | 33.5 | 18.27 | `_int_free_merge_chunk` |
| 827.0 | 827.0 | 0.0 | 22.00 | `<bacnet_device_core::read::rpm::RpmPlan>::read::<bacnet_server::device_view::DeviceReadContext, bacnet_server::server::requests::confirmed_response::read_proper` |
| 825.0 | 825.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 806.9 | 801.6 | -5.3 | 13.02 | `_int_realloc` |
| 804.9 | 804.9 | -0.0 | 17.02 | `<std::sys::pal::unix::time::Timespec>::now` |
| 798.0 | 798.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 761.4 | 752.6 | -8.8 | 2.45 | `malloc_consolidate` |
| 720.0 | 720.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::encode_property_value` |
| 720.0 | 720.0 | 0.0 | 20.00 | `<bacnet_objects::database::ObjectDatabase>::get` |
| 710.0 | 710.0 | 0.0 | 40.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 696.0 | 696.0 | 0.0 | 10.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 690.0 | 690.0 | 0.0 | 115.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 688.0 | 688.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>` |
| 670.1 | 729.0 | 58.9 | 18.27 | `_int_free_create_chunk` |
| 636.5 | 636.5 | 0.0 | 7.01 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 603.0 | 603.0 | 0.0 | 2.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 585.0 | 585.0 | 0.0 | 39.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 550.9 | 550.9 | -0.1 | 9.02 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |
| 514.0 | 514.0 | 0.0 | 12.00 | `bacnet_transport::udp_metadata::unix::recv` |
| 490.0 | 490.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_object_id` |
| 487.3 | 486.0 | -1.3 | 9.03 | `<bacnet_services::rpm::ReadPropertyMultipleACK>::decode` |
| 476.4 | 488.7 | 12.2 | 22.72 | `unlink_chunk.isra.0` |
| 461.1 | 461.0 | -0.1 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 452.6 | 452.6 | 0.0 | 5.01 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 450.0 | 450.0 | 0.0 | 1.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 448.2 | 448.4 | 0.2 | 9.99 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 431.0 | 431.0 | 0.0 | 1.00 | `bacnet_encoding::constructed::read_access::encode_read_access_specification` |
| 418.0 | 418.0 | 0.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 410.0 | 410.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 410.0 | 410.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::expect_opening` |
| 392.0 | 392.0 | 0.0 | 4.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 384.5 | 384.5 | 0.0 | 12.02 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.0 | 379.0 | 0.0 | 11.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 375.1 | 375.3 | 0.2 | 9.44 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 670.1 | 729.0 | 58.9 | `_int_free_create_chunk` |
| 6,767.2 | 6,727.5 | -39.7 | `_int_malloc` |
| 851.5 | 885.0 | 33.5 | `_int_free_merge_chunk` |
| 0.0 | 29.5 | 29.5 | `_int_memalign` |
| 82.0 | 111.5 | 29.5 | `_mid_memalign.isra.0` |
| 8,731.3 | 8,714.4 | -16.9 | `_int_free` |
| 1,125.8 | 1,111.4 | -14.4 | `bacnet_encoding::tags::extract_context_value` |
| 4,736.0 | 4,722.6 | -13.5 | `free` |
| 476.4 | 488.7 | 12.2 | `unlink_chunk.isra.0` |
| 761.4 | 752.6 | -8.8 | `malloc_consolidate` |
| 209.9 | 215.8 | 5.9 | `<alloc::raw_vec::RawVec<bacnet_types::constructed::read_access::PropertyReference>>::grow_one` |
| 806.9 | 801.6 | -5.3 | `_int_realloc` |
| 1,785.4 | 1,780.7 | -4.7 | `realloc` |
| 6,100.7 | 6,105.2 | 4.6 | `malloc` |
| 29.4 | 33.4 | 4.0 | `_int_free_maybe_consolidate.part.0` |
| 18.5 | 20.8 | 2.3 | `alloc_perturb` |
| 4,404.1 | 4,406.1 | 2.0 | `__GI_memcpy` |
| 2,576.5 | 2,578.1 | 1.6 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 487.3 | 486.0 | -1.3 | `<bacnet_services::rpm::ReadPropertyMultipleACK>::decode` |
| 263.8 | 265.0 | 1.2 | `<alloc::raw_vec::RawVec<bacnet_services::rpm::ReadResultElement>>::grow_one` |
| 147.2 | 148.4 | 1.2 | `__aarch64_swp8_acq` |
| 2,132.1 | 2,133.0 | 0.8 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 111.9 | 112.3 | 0.4 | `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv` |
| 92.9 | 93.2 | 0.3 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 448.2 | 448.4 | 0.2 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 1,435.9 | 1,435.7 | -0.2 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 88.6 | 88.4 | -0.2 | `<core::future::poll_fn::PollFn<<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}::{closure#0}> as core::futur` |
| 375.1 | 375.3 | 0.2 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 461.1 | 461.0 | -0.1 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 139.3 | 139.4 | 0.1 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 3,302.1 | 3,302.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,703.2 | 1,703.2 | -0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,638.6 | 1,638.6 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,566.7 | 1,566.7 | -0.0 | `library/core/src/ptr/mod.rs:847` |
| 1,545.6 | 1,546.3 | 0.7 | `./malloc/./malloc/malloc.c:4518` |
| 1,460.0 | 1,460.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,240.0 | 1,240.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 919.0 | 919.0 | 0.0 | `library/core/src/result.rs:0` |
| 839.1 | 846.8 | 7.8 | `./malloc/./malloc/malloc.c:1336` |
| 772.8 | 773.1 | 0.3 | `./malloc/./malloc/malloc.c:4495` |
| 767.8 | 768.1 | 0.3 | `./malloc/./malloc/malloc.c:4527` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 639.0 | 639.3 | 0.3 | `./malloc/./malloc/malloc.c:4547` |
| 606.7 | 607.2 | 0.5 | `./malloc/./malloc/malloc.c:4658` |
| 604.6 | 605.8 | 1.2 | `./malloc/./malloc/arena.c:153` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 558.0 | 558.0 | 0.0 | `library/core/src/num/uint_macros.rs:3212` |
| 515.2 | 515.4 | 0.2 | `./malloc/./malloc/malloc.c:4505` |
| 515.2 | 515.4 | 0.2 | `./malloc/./malloc/malloc.c:4510` |
| 487.8 | 487.7 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:31` |
| 453.7 | 453.7 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 445.0 | 445.0 | 0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 410.6 | 410.6 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 409.7 | 409.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 390.0 | 385.7 | -4.3 | `./malloc/./malloc/malloc.c:3163` |
| 390.0 | 390.0 | 0.0 | `library/core/src/intrinsics/mod.rs:478` |
| 375.0 | 375.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 367.0 | 367.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 364.0 | 364.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 362.1 | 362.1 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 356.1 | 356.1 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 341.7 | 341.6 | -0.1 | `./malloc/./malloc/malloc.c:1461` |
| 340.0 | 340.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 334.0 | 334.0 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 325.0 | 325.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 315.0 | 315.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:523` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:672` |
| 308.0 | 308.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:159` |
| 306.4 | 306.4 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 305.5 | 309.4 | 3.9 | `./malloc/./malloc/malloc.c:1345` |
