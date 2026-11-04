# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,399.2 | 1,269.2 | -130.0 | 20.00 | `_int_free` |
| 918.2 | 845.1 | -73.0 | 20.00 | `free` |
| 918.1 | 810.1 | -108.0 | 17.00 | `malloc` |
| 865.4 | 865.4 | -0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 843.0 | 845.4 | 2.4 | 11.14 | `<std::sys::pal::unix::time::Timespec>::now` |
| 831.0 | 802.0 | -29.0 | 45.00 | `__GI_memcpy` |
| 698.0 | 697.7 | -0.3 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 534.0 | 532.0 | -2.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 503.0 | 451.0 | -52.0 | 18.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 473.1 | 473.1 | -0.0 | 91.00 | `__aarch64_cas4_acq` |
| 468.0 | 468.7 | 0.7 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 451.0 | 451.0 | -0.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 436.9 | 417.0 | -19.9 | 87.38 | `__aarch64_ldadd8_relax` |
| 424.6 | 424.4 | -0.2 | 1.33 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 413.0 | 545.0 | 132.0 | 8.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 398.6 | 398.6 | 0.0 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 378.6 | 378.5 | -0.2 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.1 | 378.4 | 0.3 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 357.2 | 357.0 | -0.2 | 11.16 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.5 | 346.5 | -0.1 | 68.90 | `__aarch64_swp4_rel` |
| 338.1 | 338.0 | -0.1 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.3 | 319.2 | -0.1 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 317.8 | 317.9 | 0.1 | 5.99 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 281.3 | 281.3 | -0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 280.4 | 280.4 | -0.1 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.4 | 274.4 | 0.0 | 2.00 | `_int_malloc` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.7 | 250.8 | 0.1 | 5.32 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 234.0 | 234.0 | 0.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 230.2 | 230.2 | 0.0 | 46.04 | `__aarch64_cas8_acq_rel` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 213.8 | 213.8 | -0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.3 | 213.4 | 0.0 | 42.67 | `__aarch64_ldadd8_rel` |
| 213.0 | 213.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 211.9 | 211.6 | -0.3 | 2.07 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.2 | 204.1 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 202.0 | 98.0 | -104.0 | 2.00 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 195.0 | 196.9 | 1.9 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.2 | 193.2 | -0.1 | 8.05 | `__pthread_enable_asynccancel` |
| 192.0 | 0.0 | -192.0 | 4.00 | `bacnet_transport::bvll::encode_bvll` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 177.1 | 177.1 | -0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 164.1 | 164.1 | 0.0 | 11.72 | `tokio::runtime::task::waker::clone_waker` |
| 162.4 | 162.4 | -0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 192.0 | 0.0 | -192.0 | `bacnet_transport::bvll::encode_bvll` |
| 0.0 | 180.0 | 180.0 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 413.0 | 545.0 | 132.0 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 1,399.2 | 1,269.2 | -130.0 | `_int_free` |
| 918.1 | 810.1 | -108.0 | `malloc` |
| 202.0 | 98.0 | -104.0 | `<bacnet_transport::bip::socket::SendTo as core::future::future::Future>::poll` |
| 74.0 | 0.0 | -74.0 | `<bytes::bytes_mut::BytesMut as bacnet_bip_codec::sink::BvllSink>::write` |
| 918.2 | 845.1 | -73.0 | `free` |
| 70.0 | 0.0 | -70.0 | `sendto` |
| 0.0 | 54.0 | 54.0 | `sendmsg` |
| 503.0 | 451.0 | -52.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 831.0 | 802.0 | -29.0 | `__GI_memcpy` |
| 22.0 | 0.0 | -22.0 | `core::ptr::drop_glue::<<bacnet_transport::bip::node::NodeHandle>::send_unicast::{closure#0}>` |
| 436.9 | 417.0 | -19.9 | `__aarch64_ldadd8_relax` |
| 0.0 | 14.0 | 14.0 | `core::ptr::drop_glue::<<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu_with_data_attributes::{closure#0}>` |
| 11.0 | 0.0 | -11.0 | `core::ptr::drop_glue::<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}>` |
| 85.0 | 75.0 | -10.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 48.0 | 51.7 | 3.6 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 843.0 | 845.4 | 2.4 | `<std::sys::pal::unix::time::Timespec>::now` |
| 534.0 | 532.0 | -2.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 76.0 | 78.0 | 2.0 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::send_confirmed_target_apdu::{closure#0}` |
| 195.0 | 196.9 | 1.9 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 57.0 | 56.0 | -1.0 | `<bacnet_network::layer::NetworkLayer<bacnet_transport::bip::BipTransport>>::send_apdu::{closure#0}` |
| 127.3 | 128.0 | 0.7 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 468.0 | 468.7 | 0.7 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 136.3 | 136.8 | 0.6 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 96.2 | 96.6 | 0.4 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 378.1 | 378.4 | 0.3 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 211.9 | 211.6 | -0.3 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 698.0 | 697.7 | -0.3 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 836.0 | 802.0 | -34.0 | `library/core/src/ptr/mod.rs:573` |
| 679.4 | 663.5 | -15.9 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 586.3 | 579.3 | -7.0 | `library/core/src/ptr/mod.rs:847` |
| 536.1 | 536.1 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 383.7 | 381.7 | -2.1 | `library/core/src/sync/atomic.rs:3994` |
| 351.1 | 329.2 | -21.9 | `library/core/src/sync/atomic.rs:4029` |
| 291.6 | 291.5 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.4 | 291.4 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 259.5 | 259.4 | -0.2 | `library/std/src/sys/pal/unix/time.rs:107` |
| 255.6 | 255.6 | -0.0 | `library/std/src/panicking.rs:465` |
| 240.0 | 216.0 | -24.0 | `./malloc/./malloc/malloc.c:4518` |
| 218.6 | 218.7 | 0.1 | `library/core/src/cell.rs:558` |
| 218.5 | 218.5 | 0.0 | `library/core/src/mem/mod.rs:976` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 169.9 | 165.9 | -4.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 158.2 | 158.2 | 0.1 | `library/core/src/sync/atomic.rs:4104` |
| 158.2 | 158.3 | 0.2 | `library/core/src/sync/atomic.rs:3995` |
| 157.5 | 151.4 | -6.0 | `library/core/src/ptr/non_null.rs:454` |
| 156.6 | 156.6 | -0.0 | `???:0` |
| 153.6 | 153.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 153.0 | 135.0 | -18.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 152.4 | 148.4 | -4.0 | `library/core/src/option.rs:743` |
| 142.9 | 142.9 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 141.5 | 141.7 | 0.2 | `library/core/src/cell.rs:2437` |
| 134.0 | 134.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 130.8 | 130.7 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 126.0 | 118.0 | -8.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 120.0 | 108.0 | -12.0 | `./malloc/./malloc/malloc.c:4495` |
| 120.0 | 108.0 | -12.0 | `./malloc/./malloc/malloc.c:4527` |
| 112.0 | 100.0 | -12.0 | `library/core/src/result.rs:2177` |
| 102.0 | 90.0 | -12.0 | `./malloc/./malloc/malloc.c:3313` |
| 102.0 | 90.0 | -12.0 | `./malloc/./malloc/malloc.c:3354` |
| 102.0 | 90.0 | -12.0 | `./malloc/./malloc/malloc.c:3294` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:4658` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:3359` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:3392` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:3402` |
| 100.0 | 90.0 | -10.0 | `./malloc/./malloc/malloc.c:4547` |
| 100.0 | 90.0 | -10.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 99.7 | 99.9 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.2 | 99.2 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 95.0 | 87.0 | -8.0 | `./malloc/./malloc/malloc.c:1336` |
| 94.1 | 94.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 85.0 | 75.0 | -10.0 | `./malloc/./malloc/malloc.c:3311` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.0 | 81.0 | 0.0 | `library/core/src/option.rs:2439` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:3368` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:4505` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:3371` |
| 80.0 | 72.0 | -8.0 | `./malloc/./malloc/malloc.c:4510` |
| 79.5 | 79.6 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 79.0 | 79.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 77.6 | 77.6 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 77.0 | 76.9 | -0.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 75.0 | 70.0 | -5.0 | `./malloc/./malloc/arena.c:147` |
| 74.6 | 74.6 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
