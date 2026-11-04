# wp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,748.9 | 1,619.9 | -129.0 | 26.00 | `_int_free` |
| 1,651.0 | 1,621.0 | -30.0 | 52.00 | `__GI_memcpy` |
| 1,247.0 | 1,139.1 | -108.0 | 23.00 | `malloc` |
| 1,227.7 | 1,155.3 | -72.4 | 26.00 | `free` |
| 873.6 | 873.5 | -0.1 | 12.14 | `<std::sys::pal::unix::time::Timespec>::now` |
| 865.4 | 865.5 | 0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 750.3 | 750.3 | 0.0 | 7.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 698.0 | 697.8 | -0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 659.0 | 607.0 | -52.0 | 24.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 572.0 | 572.0 | 0.0 | 25.00 | `bacnet_encoding::tags::decode_tag` |
| 567.4 | 567.5 | 0.1 | 14.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 539.2 | 539.3 | 0.1 | 7.00 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::primitives::ObjectIdentifier>` |
| 534.0 | 532.0 | -2.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 512.0 | 491.9 | -20.0 | 102.40 | `__aarch64_ldadd8_relax` |
| 505.1 | 505.1 | -0.0 | 97.01 | `__aarch64_cas4_acq` |
| 483.2 | 483.5 | 0.3 | 8.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}` |
| 479.4 | 462.7 | -16.8 | 3.19 | `_int_malloc` |
| 468.4 | 468.1 | -0.3 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 427.5 | 427.1 | -0.4 | 1.28 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 417.3 | 417.3 | 0.0 | 5.00 | `<tokio::sync::batch_semaphore::Acquire as core::future::future::Future>::poll` |
| 413.0 | 545.0 | 132.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.6 | 398.5 | -0.1 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 380.2 | 380.1 | -0.1 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.9 | 378.5 | -0.4 | 9.45 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 378.5 | 378.5 | -0.0 | 74.91 | `__aarch64_swp4_rel` |
| 357.1 | 357.1 | 0.0 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::tags::extract_context_value` |
| 339.0 | 339.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 338.1 | 338.1 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.1 | 319.1 | 0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 317.9 | 317.9 | -0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 293.0 | 293.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 280.4 | 280.5 | 0.0 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 280.0 | 280.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 252.0 | 252.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_optional_ctx::<u64, bacnet_encoding::constructed::tagged::decode_ctx_unsigned<u64>>` |
| 251.0 | 250.9 | -0.1 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 250.3 | 250.3 | -0.0 | 50.06 | `__aarch64_cas8_acq_rel` |
| 242.0 | 242.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 233.4 | 233.4 | -0.0 | 46.68 | `__aarch64_ldadd8_rel` |
| 228.0 | 228.0 | 0.0 | 9.00 | `<bacnet_services::write_property::WritePropertyRequest>::decode` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 216.0 | 216.0 | 0.0 | 3.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 212.3 | 212.4 | 0.0 | 3.00 | `<tokio::sync::rwlock::RwLock<bacnet_objects::database::ObjectDatabase>>::write::{closure#0}` |
| 210.9 | 211.0 | 0.1 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 204.2 | 204.2 | 0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 98.0 | -104.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 197.2 | 197.2 | 0.0 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.2 | 193.2 | 0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 0.0 | -192.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 192.0 | 0.0 | -192.0 | `bacnet_transport::bvll::encode_bvll` |
| 0.0 | 180.0 | 180.0 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 413.0 | 545.0 | 132.0 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 1,748.9 | 1,619.9 | -129.0 | `_int_free` |
| 1,247.0 | 1,139.1 | -108.0 | `malloc` |
| 202.0 | 98.0 | -104.0 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 74.0 | 0.0 | -74.0 | `<bytes::bytes_mut::BytesMut as bacnet_bip_codec::sink::BvllSink>::write` |
| 1,227.7 | 1,155.3 | -72.4 | `free` |
| 70.0 | 0.0 | -70.0 | `sendto` |
| 0.0 | 54.0 | 54.0 | `sendmsg` |
| 659.0 | 607.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 1,651.0 | 1,621.0 | -30.0 | `__GI_memcpy` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<<bacnet_transport::bip::node::NodeHandle>::send_unicast::{closure#0}>` |
| 512.0 | 491.9 | -20.0 | `__aarch64_ldadd8_relax` |
| 479.4 | 462.7 | -16.8 | `_int_malloc` |
| 48.5 | 32.7 | -15.8 | `_int_free_maybe_consolidate.part.0` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu_with_data_attributes::{closure#0}>` |
| 11.0 | 0.0 | -11.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}>` |
| 115.0 | 105.0 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 14.2 | 7.0 | -7.2 | `unlink_chunk.isra.0` |
| 146.1 | 139.1 | -7.1 | `malloc_consolidate` |
| 22.3 | 26.7 | 4.5 | `_int_free_create_chunk` |
| 534.0 | 532.0 | -2.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 76.0 | 78.0 | 2.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}` |
| 57.0 | 56.0 | -1.0 | `<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu::{closure#0}` |
| 6.0 | 5.4 | -0.6 | `alloc_perturb` |
| 122.8 | 123.4 | 0.6 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 427.5 | 427.1 | -0.4 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 378.9 | 378.5 | -0.4 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 136.7 | 136.4 | -0.4 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 989.0 | 955.0 | -34.0 | `library/core/src/ptr/mod.rs:573` |
| 838.3 | 831.3 | -7.0 | `library/core/src/ptr/mod.rs:847` |
| 755.6 | 739.5 | -16.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 572.2 | 572.2 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 435.2 | 413.1 | -22.0 | `library/core/src/sync/atomic.rs:4029` |
| 400.7 | 398.7 | -2.0 | `library/core/src/sync/atomic.rs:3994` |
| 312.0 | 288.0 | -24.0 | `./malloc/./malloc/malloc.c:4518` |
| 311.4 | 311.4 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 307.6 | 307.6 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 276.4 | 276.5 | 0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 271.6 | 271.6 | -0.0 | `library/std/src/panicking.rs:465` |
| 257.4 | 253.4 | -4.0 | `library/core/src/option.rs:743` |
| 234.7 | 234.7 | -0.0 | `library/core/src/cell.rs:558` |
| 229.0 | 217.0 | -12.0 | `library/core/src/result.rs:2177` |
| 224.5 | 224.5 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 222.6 | 222.6 | 0.0 | `???:0` |
| 210.0 | 210.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 207.0 | 189.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 188.9 | 184.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 174.3 | 174.2 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 165.6 | 165.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 164.3 | 158.3 | -6.0 | `library/core/src/ptr/non_null.rs:454` |
| 162.4 | 162.3 | -0.0 | `library/core/src/sync/atomic.rs:3995` |
| 158.0 | 150.0 | -8.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 156.0 | 144.0 | -12.0 | `./malloc/./malloc/malloc.c:4495` |
| 155.0 | 155.0 | -0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 151.0 | 139.0 | -12.0 | `./malloc/./malloc/malloc.c:4527` |
| 150.3 | 150.4 | 0.0 | `library/core/src/num/uint_macros.rs:2652` |
| 144.7 | 144.7 | 0.0 | `library/core/src/cell.rs:2437` |
| 143.1 | 143.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 138.8 | 138.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 138.0 | 126.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 138.0 | 126.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 138.0 | 126.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 130.0 | 120.0 | -10.0 | `./malloc/./malloc/malloc.c:4658` |
| 130.0 | 120.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 130.0 | 120.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 130.0 | 120.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 130.0 | 120.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 129.0 | 121.0 | -8.0 | `./malloc/./malloc/malloc.c:1336` |
| 128.0 | 128.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 125.0 | 115.0 | -10.0 | `./malloc/./malloc/malloc.c:4547` |
| 115.0 | 105.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 113.0 | 107.0 | -6.0 | `library/core/src/result.rs:0` |
| 106.1 | 106.1 | 0.0 | `library/core/src/ptr/mod.rs:1755` |
| 105.0 | 105.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 104.0 | 96.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 104.0 | 96.0 | -8.0 | `./malloc/./malloc/malloc.c:4505` |
| 104.0 | 96.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 104.0 | 96.0 | -8.0 | `./malloc/./malloc/malloc.c:4510` |
| 103.9 | 98.3 | -5.6 | `./malloc/./malloc/arena.c:147` |
| 99.8 | 99.8 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:173` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:167` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:169` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:166` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:168` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:170` |
| 99.0 | 99.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:172` |
