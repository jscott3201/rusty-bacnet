# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,269.2 | 1,269.1 | -0.1 | 18.00 | `_int_free` |
| 865.5 | 865.4 | -0.0 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 846.4 | 721.2 | -125.2 | 11.15 | `<std::sys::pal::unix::time::Timespec>::now` |
| 845.2 | 845.1 | -0.1 | 18.00 | `free` |
| 810.1 | 810.1 | -0.0 | 15.00 | `malloc` |
| 802.0 | 802.0 | 0.0 | 43.00 | `__GI_memcpy` |
| 698.0 | 697.9 | -0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 532.0 | 736.0 | 204.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 473.2 | 473.1 | -0.1 | 91.02 | `__aarch64_cas4_acq` |
| 468.4 | 468.4 | 0.0 | 9.98 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 451.0 | 451.0 | 0.0 | 16.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 451.0 | 451.0 | 0.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 424.4 | 424.4 | -0.0 | 1.35 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 416.9 | 417.0 | 0.1 | 83.39 | `__aarch64_ldadd8_relax` |
| 398.6 | 398.5 | -0.1 | 7.76 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 379.0 | 378.2 | -0.8 | 4.10 | `<tokio::runtime::time::Driver>::park_internal` |
| 378.3 | 378.8 | 0.5 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 357.4 | 293.1 | -64.3 | 11.17 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 346.6 | 346.5 | -0.1 | 68.92 | `__aarch64_swp4_rel` |
| 338.2 | 338.0 | -0.2 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.4 | 319.2 | -0.2 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 317.8 | 317.9 | 0.1 | 5.99 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 291.0 | 291.0 | -0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 281.4 | 281.3 | -0.1 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 280.5 | 280.4 | -0.1 | 5.01 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.3 | 274.2 | -0.1 | 2.00 | `_int_malloc` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 250.8 | 250.9 | 0.2 | 5.32 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 234.0 | 0.0 | -234.0 | 2.00 | `<bacnet_time::host::HostClock>::now` |
| 230.3 | 230.3 | -0.1 | 46.06 | `__aarch64_cas8_acq_rel` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 214.0 | 214.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | 0.0 | 42.67 | `__aarch64_ldadd8_rel` |
| 212.6 | 211.7 | -1.0 | 2.07 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.2 | 204.2 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | -0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 197.4 | 195.8 | -1.6 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.3 | 193.2 | -0.1 | 8.05 | `__pthread_enable_asynccancel` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 180.0 | 180.0 | 0.0 | 2.00 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 177.2 | 177.1 | -0.1 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 164.0 | 164.1 | 0.1 | 11.72 | `tokio::runtime::task::waker::clone_waker` |
| 162.6 | 162.5 | -0.1 | 4.02 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 159.0 | 159.0 | 0.0 | 2.00 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 234.0 | 0.0 | -234.0 | `<bacnet_time::host::HostClock>::now` |
| 532.0 | 736.0 | 204.0 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 846.4 | 721.2 | -125.2 | `<std::sys::pal::unix::time::Timespec>::now` |
| 357.4 | 293.1 | -64.3 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 152.8 | 132.6 | -20.1 | `clock_gettime@@GLIBC_2.17` |
| 4.0 | 0.0 | -4.0 | `<bacnet_transport::runtime_clock::tokio_clock::{closure#0} as core::ops::function::FnOnce<()>>::call_once` |
| 51.9 | 48.4 | -3.6 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 197.4 | 195.8 | -1.6 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 129.2 | 128.0 | -1.2 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 212.6 | 211.7 | -1.0 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 379.0 | 378.2 | -0.8 | `<tokio::runtime::time::Driver>::park_internal` |
| 101.1 | 101.8 | 0.8 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 49.2 | 49.9 | 0.7 | `<tokio::runtime::time::wheel::Wheel>::remove` |
| 82.8 | 83.4 | 0.6 | `<tokio::task::join_set::JoinSet<()>>::poll_join_next` |
| 378.3 | 378.8 | 0.5 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 136.5 | 136.9 | 0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 105.3 | 105.0 | -0.3 | `<tokio::runtime::scheduler::multi_thread::worker::Core>::maintenance` |
| 1.1 | 0.8 | -0.3 | `<tokio::runtime::scheduler::multi_thread::idle::Idle>::worker_to_notify` |
| 96.2 | 96.5 | 0.3 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 93.3 | 93.0 | -0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 133.6 | 133.9 | 0.2 | `<tokio::sync::notify::Notified as core::ops::drop::Drop>::drop` |
| 88.6 | 88.8 | 0.2 | `<tokio::runtime::park::Inner>::park` |
| 81.6 | 81.8 | 0.2 | `counts::main::{closure#0}` |
| 319.4 | 319.2 | -0.2 | `<tokio::runtime::io::driver::Driver>::turn` |
| 0.8 | 0.6 | -0.2 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |
| 250.8 | 250.9 | 0.2 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 121.4 | 121.6 | 0.2 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |
| 338.2 | 338.0 | -0.2 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 0.7 | 0.6 | -0.1 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&bacnet_types::enums::object_type::ObjectType>` |
| 51.1 | 51.3 | 0.1 | `tokio::runtime::park::clone` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 803.0 | 803.0 | -0.0 | `library/core/src/ptr/mod.rs:573` |
| 663.6 | 663.6 | 0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 579.3 | 579.3 | -0.1 | `library/core/src/ptr/mod.rs:847` |
| 536.3 | 536.2 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 381.9 | 381.7 | -0.2 | `library/core/src/sync/atomic.rs:3994` |
| 329.1 | 329.2 | 0.1 | `library/core/src/sync/atomic.rs:4029` |
| 291.7 | 291.6 | -0.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.4 | 291.4 | -0.0 | `library/core/src/sync/atomic.rs:4083` |
| 259.7 | 225.5 | -34.2 | `library/std/src/sys/pal/unix/time.rs:107` |
| 255.6 | 255.6 | -0.0 | `library/std/src/panicking.rs:465` |
| 218.8 | 218.7 | -0.0 | `library/core/src/cell.rs:558` |
| 218.6 | 218.5 | -0.1 | `library/core/src/mem/mod.rs:976` |
| 216.0 | 216.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 165.9 | 165.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 158.4 | 158.2 | -0.1 | `library/core/src/sync/atomic.rs:3995` |
| 158.3 | 158.3 | -0.0 | `library/core/src/sync/atomic.rs:4104` |
| 156.7 | 156.6 | -0.1 | `???:0` |
| 153.6 | 153.6 | -0.0 | `library/core/src/sync/atomic.rs:4013` |
| 151.5 | 151.6 | 0.1 | `library/core/src/ptr/non_null.rs:454` |
| 148.4 | 148.4 | -0.0 | `library/core/src/option.rs:743` |
| 143.0 | 143.0 | 0.0 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 141.7 | 141.6 | -0.2 | `library/core/src/cell.rs:2437` |
| 135.0 | 135.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 134.1 | 134.1 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 130.8 | 130.8 | -0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 118.0 | 118.0 | 0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 108.0 | 108.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 108.0 | 108.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 99.8 | 99.8 | 0.0 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.2 | 99.2 | -0.0 | `library/core/src/mem/mod.rs:975` |
| 98.0 | 98.0 | -0.0 | `library/core/src/slice/cmp.rs:157` |
| 97.0 | 97.0 | 0.0 | `library/core/src/result.rs:2177` |
| 94.1 | 94.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 90.0 | 90.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 80.9 | 81.0 | 0.1 | `library/core/src/option.rs:2439` |
| 79.5 | 79.7 | 0.2 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 79.2 | 79.1 | -0.1 | `library/core/src/ptr/mod.rs:1963` |
| 77.7 | 77.6 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 77.0 | 55.0 | -22.0 | `library/std/src/sys/pal/unix/time.rs:25` |
| 75.0 | 75.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 74.6 | 74.6 | -0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
| 74.2 | 74.2 | -0.0 | `library/core/src/option.rs:0` |
| 74.0 | 74.0 | 0.0 | `crates/bacnet-network/src/layer_receive.rs:89` |
| 74.0 | 74.0 | 0.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 73.3 | 73.4 | 0.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:686` |
| 72.9 | 72.9 | -0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:208` |
