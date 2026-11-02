# rpm / mt: self instructions per request

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 9,067.7 | 9,078.0 | 10.3 | 237.07 | `_int_free` |
| 6,367.5 | 6,362.1 | -5.4 | 131.13 | `_int_malloc` |
| 6,205.2 | 6,204.6 | -0.6 | 118.00 | `malloc` |
| 4,981.5 | 4,983.8 | 2.2 | 151.03 | `free` |
| 4,383.3 | 4,384.6 | 1.3 | 192.24 | `__GI_memcpy` |
| 3,350.0 | 3,350.0 | 0.0 | 160.00 | `bacnet_encoding::tags::decode_tag` |
| 3,249.9 | 3,249.9 | 0.0 | 104.00 | `<bytes::bytes_mut::BytesMut>::reserve_inner` |
| 2,989.0 | 2,989.0 | 0.0 | 114.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 2,579.8 | 2,580.0 | 0.2 | 91.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 2,132.7 | 2,132.7 | -0.0 | 40.00 | `bacnet_encoding::constructed::read_access::decode_read_access_specification` |
| 1,961.0 | 1,961.0 | -0.0 | 71.00 | `realloc` |
| 1,654.0 | 1,654.0 | 0.0 | 30.00 | `<bacnet_device_core::read::rpm::Scratch>::append` |
| 1,647.2 | 1,647.2 | 0.1 | 329.43 | `__aarch64_ldadd8_relax` |
| 1,621.9 | 1,621.9 | 0.0 | 80.11 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 1,620.8 | 1,620.9 | 0.0 | 52.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 1,561.4 | 1,561.4 | 0.0 | 20.02 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 1,456.9 | 1,458.0 | 1.0 | 20.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 1,210.0 | 1,210.0 | 0.0 | 20.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 1,187.1 | 1,161.2 | -25.9 | 16.85 | `malloc_consolidate` |
| 1,164.0 | 1,164.0 | 0.0 | 30.00 | `bacnet_encoding::tags::extract_context_value` |
| 1,160.0 | 1,160.0 | 0.0 | 30.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u32, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u32>>` |
| 990.0 | 990.0 | 0.0 | 20.00 | `<bacnet_objects::analog::input::AnalogInputObject as bacnet_objects::traits::BACnetObject>::read_property` |
| 960.0 | 960.0 | 0.0 | 20.00 | `bacnet_encoding::primitives::scalar::encode_ctx_unsigned` |
| 894.0 | 894.0 | 0.0 | 20.00 | `bacnet_encoding::constructed::read_access::encode_property_reference` |
| 885.8 | 877.1 | -8.8 | 14.84 | `_int_realloc` |
| 839.6 | 840.3 | 0.7 | 32.00 | `<bacnet_device_core::read::rpm::RpmPlan>::read::<bacnet_server::device_view::DeviceReadContext, bacnet_server::server::requests::confirmed_response::read_proper` |
| 831.7 | 832.4 | 0.7 | 17.31 | `<std::sys::pal::unix::time::Timespec>::now` |
| 825.0 | 825.0 | 0.0 | 25.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 818.6 | 832.3 | 13.8 | 18.07 | `_int_free_merge_chunk` |
| 798.0 | 798.0 | 0.0 | 16.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 724.7 | 724.7 | -0.0 | 40.00 | `bacnet_encoding::primitives::encode_property_value` |
| 720.0 | 720.0 | 0.0 | 20.00 | `<bacnet_objects::database::ObjectDatabase>::get` |
| 710.0 | 710.0 | 0.0 | 40.00 | `bacnet_device_core::read::rpm::element::<bacnet_device_core::read::rpm::read_row<bacnet_server::device_view::DeviceReadContext>::{closure#0}>` |
| 707.5 | 709.2 | 1.7 | 130.08 | `__aarch64_cas4_acq` |
| 696.0 | 696.0 | 0.0 | 14.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 693.5 | 693.2 | -0.2 | 115.58 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 690.0 | 690.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 688.0 | 688.0 | 0.0 | 8.00 | `<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>` |
| 672.3 | 684.0 | 11.7 | 24.56 | `_int_free_create_chunk` |
| 636.8 | 636.8 | 0.0 | 14.02 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 603.0 | 603.0 | 0.0 | 12.00 | `<alloc::vec::Vec<bacnet_types::constructed::read_access::ReadAccessSpecification> as core::clone::Clone>::clone` |
| 585.0 | 585.0 | 0.0 | 39.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 580.9 | 582.6 | 1.7 | 107.98 | `__aarch64_swp4_rel` |
| 514.0 | 514.0 | 0.0 | 18.00 | `bacnet_transport::udp_metadata::unix::recv` |
| 490.0 | 490.0 | 0.0 | 10.00 | `bacnet_encoding::primitives::scalar::encode_ctx_object_id` |
| 486.0 | 486.0 | 0.0 | 9.00 | `<bacnet_services::rpm::ReadPropertyMultipleACK>::decode` |
| 461.0 | 461.0 | 0.0 | 16.00 | `<alloc::raw_vec::RawVecInner>::grow_amortized` |
| 455.7 | 453.4 | -2.3 | 22.35 | `unlink_chunk.isra.0` |
| 453.0 | 453.0 | 0.0 | 20.04 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 450.0 | 450.0 | 0.0 | 2.00 | `<bacnet_device_core::read::rpm::RpmPlan>::live_lists` |
| 447.9 | 449.1 | 1.1 | 16.94 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 431.0 | 431.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::read_access::encode_read_access_specification` |
| 418.0 | 418.0 | 0.0 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 410.0 | 410.0 | 0.0 | 10.00 | `<bacnet_server::device_view::DeviceReadView as bacnet_objects::traits::BACnetObject>::read_property` |
| 410.0 | 410.0 | 0.0 | 8.00 | `bacnet_encoding::apdu::decode_apdu` |
| 410.0 | 410.0 | 0.0 | 10.00 | `bacnet_encoding::constructed::tagged::expect_opening` |
| 392.0 | 392.0 | 0.0 | 8.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 380.0 | 380.0 | 0.0 | 20.00 | `core::ptr::drop_glue::<bacnet_types::primitives::PropertyValue>` |
| 379.0 | 379.0 | 0.0 | 16.00 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>` |
| 377.6 | 377.6 | -0.1 | 12.34 | `<tokio::runtime::time::wheel::Wheel>::next_expiration` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 1,187.1 | 1,161.2 | -25.9 | `malloc_consolidate` |
| 818.6 | 832.3 | 13.8 | `_int_free_merge_chunk` |
| 176.7 | 163.6 | -13.1 | `__aarch64_swp8_acq` |
| 672.3 | 684.0 | 11.7 | `_int_free_create_chunk` |
| 9,067.7 | 9,078.0 | 10.3 | `_int_free` |
| 885.8 | 877.1 | -8.8 | `_int_realloc` |
| 6,367.5 | 6,362.1 | -5.4 | `_int_malloc` |
| 43.2 | 47.4 | 4.2 | `_int_memalign` |
| 84.8 | 81.1 | -3.7 | `_int_free_maybe_consolidate.part.0` |
| 119.9 | 123.7 | 3.7 | `_mid_memalign.isra.0` |
| 455.7 | 453.4 | -2.3 | `unlink_chunk.isra.0` |
| 4,981.5 | 4,983.8 | 2.2 | `free` |
| 707.5 | 709.2 | 1.7 | `__aarch64_cas4_acq` |
| 580.9 | 582.6 | 1.7 | `__aarch64_swp4_rel` |
| 4,383.3 | 4,384.6 | 1.3 | `__GI_memcpy` |
| 447.9 | 449.1 | 1.1 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 1,456.9 | 1,458.0 | 1.0 | `bacnet_device_core::read::rpm::plan_specs::<bacnet_server::device_view::DeviceReadContext>::{closure#2}` |
| 111.6 | 112.5 | 0.9 | `<tokio::sync::mpsc::chan::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup, tokio::sync::mpsc::unbounded::Semaphore>>::recv` |
| 92.6 | 93.4 | 0.7 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 831.7 | 832.4 | 0.7 | `<std::sys::pal::unix::time::Timespec>::now` |
| 839.6 | 840.3 | 0.7 | `<bacnet_device_core::read::rpm::RpmPlan>::read::<bacnet_server::device_view::DeviceReadContext, bacnet_server::server::requests::confirmed_response::read_proper` |
| 6,205.2 | 6,204.6 | -0.6 | `malloc` |
| 260.1 | 259.5 | -0.6 | `<alloc::raw_vec::RawVec<bacnet_device_core::read::rpm::PlannedRow>>::grow_one` |
| 17.1 | 17.7 | 0.5 | `<tokio::runtime::task::core::Trailer>::set_waker` |
| 376.1 | 376.6 | 0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 219.5 | 219.1 | -0.5 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 80.2 | 80.6 | 0.4 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request` |
| 226.3 | 226.6 | 0.3 | `<tokio::task::coop::RestoreOnPending as core::ops::drop::Drop>::drop` |
| 234.5 | 234.7 | 0.3 | `__rustc::__rust_dealloc` |
| 77.4 | 77.7 | 0.3 | `<tokio::time::interval::Interval>::poll_tick` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 3,302.1 | 3,302.1 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 1,706.3 | 1,706.3 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 1,664.2 | 1,664.2 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 1,556.1 | 1,556.2 | 0.1 | `library/core/src/ptr/mod.rs:847` |
| 1,527.8 | 1,529.0 | 1.2 | `./malloc/./malloc/malloc.c:4518` |
| 1,460.0 | 1,460.0 | 0.0 | `library/core/src/result.rs:2177` |
| 1,240.0 | 1,240.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 1,150.0 | 1,150.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 1,017.0 | 1,017.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 1,015.8 | 1,003.1 | -12.7 | `???:0` |
| 919.0 | 919.0 | 0.0 | `library/core/src/result.rs:0` |
| 784.1 | 787.5 | 3.4 | `./malloc/./malloc/malloc.c:1336` |
| 763.9 | 764.5 | 0.6 | `./malloc/./malloc/malloc.c:4495` |
| 758.9 | 759.5 | 0.6 | `./malloc/./malloc/malloc.c:4527` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 678.0 | 678.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 631.6 | 632.1 | 0.5 | `./malloc/./malloc/malloc.c:4547` |
| 628.8 | 629.3 | 0.6 | `./malloc/./malloc/malloc.c:4658` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 580.0 | 580.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 580.0 | 580.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 565.0 | 565.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 558.0 | 558.0 | 0.0 | `library/core/src/num/uint_macros.rs:3212` |
| 553.2 | 553.3 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 533.4 | 533.0 | -0.4 | `./malloc/./malloc/arena.c:147` |
| 509.3 | 509.7 | 0.4 | `./malloc/./malloc/malloc.c:4505` |
| 509.3 | 509.7 | 0.4 | `./malloc/./malloc/malloc.c:4510` |
| 480.0 | 480.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 464.0 | 464.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 456.0 | 456.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:31` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 452.0 | 452.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 451.0 | 451.0 | -0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 449.3 | 449.0 | -0.3 | `./malloc/./malloc/arena.c:153` |
| 429.7 | 430.0 | 0.2 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 416.0 | 416.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 409.9 | 410.0 | 0.1 | `./malloc/./malloc/malloc.c:3163` |
| 396.8 | 396.8 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 394.3 | 394.3 | -0.0 | `library/core/src/sync/atomic.rs:3994` |
| 387.2 | 387.2 | -0.0 | `library/core/src/intrinsics/mod.rs:478` |
| 375.0 | 375.0 | 0.0 | `library/core/src/alloc/global.rs:0` |
| 367.0 | 367.0 | 0.0 | `library/core/src/cmp.rs:1051` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:564` |
| 364.0 | 364.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:544` |
| 364.0 | 364.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:626` |
| 353.2 | 353.2 | 0.0 | `library/core/src/ptr/mod.rs:1963` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 348.0 | 348.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 346.6 | 346.5 | -0.1 | `./malloc/./malloc/malloc.c:1461` |
| 340.0 | 340.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 335.0 | 335.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:529` |
| 334.1 | 334.1 | -0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 325.0 | 325.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 315.0 | 315.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:523` |
| 312.0 | 312.0 | 0.0 | `library/alloc/src/raw_vec/mod.rs:672` |
| 312.0 | 312.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:806` |
| 311.5 | 311.5 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
