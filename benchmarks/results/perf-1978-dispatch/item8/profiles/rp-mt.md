# rp / mt: self instructions per request

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 1,269.1 | 1,269.1 | -0.0 | 18.00 | `_int_free` |
| 865.3 | 878.8 | 13.4 | 6.00 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 845.1 | 845.1 | -0.0 | 18.00 | `free` |
| 810.0 | 810.1 | 0.0 | 15.00 | `malloc` |
| 802.0 | 802.0 | 0.0 | 43.00 | `__GI_memcpy` |
| 736.0 | 736.0 | 0.0 | 8.00 | `<bacnet_transport::bip::ingress::Listeners>::recv::{closure#0}` |
| 719.1 | 719.3 | 0.2 | 9.13 | `<std::sys::pal::unix::time::Timespec>::now` |
| 697.7 | 697.6 | -0.1 | 8.21 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 654.0 | 654.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::Readiness as core::future::future::Future>::poll` |
| 546.0 | 546.0 | 0.0 | 8.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::try_recv_from` |
| 546.0 | 546.0 | 0.0 | 12.00 | `<bacnet_transport::udp_metadata::DestinationReceiver>::recv_from::{closure#0}` |
| 545.0 | 545.0 | 0.0 | 4.00 | `<bacnet_transport::bip::BipTransport as bacnet_transport::port::TransportPort>::send_unicast::{closure#0}` |
| 473.0 | 473.0 | -0.0 | 90.98 | `__aarch64_cas4_acq` |
| 468.0 | 296.3 | -171.8 | 9.97 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 451.0 | 451.0 | 0.0 | 16.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 451.0 | 451.0 | -0.0 | 6.00 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}` |
| 424.0 | 424.0 | 0.0 | 6.00 | `<tokio::runtime::io::scheduled_io::ScheduledIo>::wake` |
| 422.3 | 423.6 | 1.3 | 1.36 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 416.9 | 399.4 | -17.5 | 83.39 | `__aarch64_ldadd8_relax` |
| 398.5 | 224.0 | -174.5 | 7.75 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 378.4 | 290.6 | -87.8 | 9.43 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 376.9 | 377.6 | 0.7 | 4.09 | `<tokio::runtime::time::Driver>::park_internal` |
| 346.4 | 346.4 | 0.0 | 68.89 | `__aarch64_swp4_rel` |
| 338.0 | 338.0 | -0.0 | 4.01 | `<tokio::sync::mpsc::list::Rx<bacnet_transport::port::ReceivedNpdu>>::pop` |
| 332.0 | 332.0 | 0.0 | 4.00 | `<nix::sys::socket::CmsgIterator as core::iter::traits::iterator::Iterator>::next` |
| 319.0 | 319.0 | -0.0 | 6.05 | `<tokio::runtime::io::driver::Driver>::turn` |
| 318.0 | 318.0 | 0.0 | 6.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::confirmed_request_inner::{closure#0}` |
| 312.0 | 312.0 | 0.0 | 2.00 | `<hashbrown::map::HashMap<(smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction, core::hash::BuildHasherDefault<bacnet_types::map_hash::MapHa` |
| 292.9 | 292.8 | -0.0 | 9.15 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 291.0 | 291.0 | 0.0 | 1.00 | `<bacnet_server::server::request_admission::Admission>::try_enter` |
| 285.0 | 285.0 | 0.0 | 3.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 284.0 | 284.0 | 0.0 | 3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bi` |
| 281.2 | 281.2 | 0.0 | 3.00 | `<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked` |
| 280.3 | 280.3 | -0.0 | 5.00 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::schedule_task` |
| 279.0 | 279.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::decode_apdu` |
| 274.3 | 274.2 | -0.1 | 2.00 | `_int_malloc` |
| 254.0 | 254.0 | 0.0 | 3.00 | `<hashbrown::raw::RawTable<((smallvec::SmallVec<[u8; 6]>, u8), bacnet_client::tsm::PendingTransaction)>>::find::<hashbrown::map::equivalent_key<(smallvec::SmallV` |
| 251.0 | 250.9 | -0.1 | 5.33 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::wait_for_confirmed_response::{closure#0}` |
| 230.2 | 212.6 | -17.5 | 46.03 | `__aarch64_cas8_acq_rel` |
| 222.0 | 222.0 | 0.0 | 8.00 | `bacnet_transport::bip::ingress::receive_loop::{closure#0}` |
| 214.0 | 214.0 | 0.0 | 2.00 | `bacnet_services::read_property::encode_reference` |
| 213.8 | 213.8 | 0.0 | 3.00 | `<bytes::bytes::Bytes>::copy_from_slice` |
| 213.4 | 213.4 | -0.0 | 42.67 | `__aarch64_ldadd8_rel` |
| 211.8 | 211.3 | -0.6 | 2.06 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 211.0 | 211.0 | 0.0 | 2.00 | `bacnet_encoding::apdu::encode_apdu` |
| 204.1 | 204.1 | -0.0 | 2.00 | `pow@@GLIBC_2.29` |
| 204.0 | 204.0 | 0.0 | 4.00 | `tokio::runtime::task::raw::poll::<bacnet_transport::bip::ingress::receive_loop::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::H` |
| 193.7 | 194.4 | 0.6 | 4.01 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 193.1 | 193.1 | -0.0 | 8.05 | `__pthread_enable_asynccancel` |
| 189.0 | 189.0 | 0.0 | 2.00 | `bacnet_server::server::request_peer::canonical_requester` |
| 183.0 | 183.0 | 0.0 | 8.00 | `bacnet_encoding::tags::decode_tag` |
| 181.0 | 181.0 | 0.0 | 2.00 | `<hashbrown::raw::RawTable<(bacnet_device_core::cov::identity::CovRecipient, usize)>>::find::<hashbrown::map::equivalent_key<bacnet_device_core::cov::identity::C` |
| 181.0 | 181.0 | -0.0 | 2.00 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::dispatch_apdu::{closure#0}` |
| 180.0 | 180.0 | 0.0 | 2.00 | `<bacnet_transport::bip::datagram::OutgoingDatagram>::try_send` |
| 177.0 | 177.0 | -0.0 | 8.05 | `__pthread_disable_asynccancel` |
| 173.0 | 173.0 | 0.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 164.0 | 115.0 | -49.0 | 11.71 | `tokio::runtime::task::waker::clone_waker` |
| 162.4 | 162.4 | 0.0 | 4.01 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 159.0 | 159.0 | -0.0 | 2.00 | `<bacnet_client::tsm::Tsm>::register_coordinated_transaction_with_progress` |
| 156.0 | 156.0 | -0.0 | 2.00 | `bacnet_client::client::response_admission::complete_terminal_response::{closure#0}` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 398.5 | 224.0 | -174.5 | `<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}` |
| 468.0 | 296.3 | -171.8 | `<tokio::sync::task::atomic_waker::AtomicWaker>::register_by_ref` |
| 117.5 | 0.0 | -117.5 | `<tokio::sync::mpsc::list::Rx<bacnet_network::layer::ReceivedNetworkControl>>::pop` |
| 378.4 | 290.6 | -87.8 | `<tokio::time::sleep::Sleep as core::future::future::Future>::poll` |
| 87.7 | 0.0 | -87.7 | `<tokio::sync::mpsc::bounded::Receiver<bacnet_network::layer::ReceivedNetworkControl>>::recv::{closure#0}` |
| 76.4 | 0.0 | -76.4 | `<tokio::time::interval::Interval>::poll_tick` |
| 0.0 | 72.0 | 72.0 | `<bacnet_client::client::lifecycle::Controls>::take` |
| 0.0 | 63.0 | 63.0 | `<core::option::Option<<bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>>::start_with_options::{closure#0}::{closure#0}::{closure#0}::{cl` |
| 164.0 | 115.0 | -49.0 | `tokio::runtime::task::waker::clone_waker` |
| 107.9 | 58.9 | -49.0 | `tokio::runtime::task::waker::drop_waker` |
| 93.2 | 62.0 | -31.2 | `<tokio::sync::mpsc::list::Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup>>::pop` |
| 416.9 | 399.4 | -17.5 | `__aarch64_ldadd8_relax` |
| 230.2 | 212.6 | -17.5 | `__aarch64_cas8_acq_rel` |
| 77.9 | 60.3 | -17.5 | `__aarch64_cas8_acq` |
| 113.3 | 95.8 | -17.5 | `__aarch64_ldadd8_acq_rel` |
| 865.3 | 878.8 | 13.4 | `<bacnet_network::layer::ingress::ApduReceiver>::poll_recv` |
| 130.3 | 128.1 | -2.3 | `<tokio::runtime::time::handle::Handle>::process_at_time` |
| 422.3 | 423.6 | 1.3 | `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, ` |
| 67.9 | 67.0 | -1.0 | `counts::main::{closure#0}` |
| 376.9 | 377.6 | 0.7 | `<tokio::runtime::time::Driver>::park_internal` |
| 193.7 | 194.4 | 0.6 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task` |
| 46.3 | 46.9 | 0.6 | `<tokio::runtime::scheduler::multi_thread::handle::Handle>::next_remote_task` |
| 211.8 | 211.3 | -0.6 | `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal` |
| 136.9 | 136.6 | -0.3 | `<tokio::sync::notify::NotifiedProject>::poll_notified` |
| 96.6 | 96.3 | -0.2 | `<bacnet_client::tsm::progress::ProgressReceiver>::changed::{closure#0}` |
| 719.1 | 719.3 | 0.2 | `<std::sys::pal::unix::time::Timespec>::now` |
| 0.2 | 0.4 | 0.2 | `<std::sys::sync::mutex::futex::Mutex>::lock_contended` |
| 101.1 | 100.9 | -0.1 | `<bacnet_server::server::notification_transactions::NotificationTransactions>::join_next::{closure#0}` |
| 697.7 | 697.6 | -0.1 | `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::start_on_heap::{closure#0}::{closure#6}::{closure#0}` |
| 121.5 | 121.6 | 0.1 | `<tokio::sync::oneshot::Receiver<bacnet_client::tsm::TsmResponse> as core::future::future::Future>::poll` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 803.0 | 803.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 663.5 | 635.4 | -28.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 579.2 | 580.7 | 1.5 | `library/core/src/ptr/mod.rs:847` |
| 536.0 | 508.0 | -28.1 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:146` |
| 381.6 | 381.6 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 329.1 | 318.6 | -10.5 | `library/core/src/sync/atomic.rs:4029` |
| 291.5 | 291.5 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:206` |
| 291.3 | 291.3 | 0.0 | `library/core/src/sync/atomic.rs:4083` |
| 255.5 | 255.5 | 0.0 | `library/std/src/panicking.rs:465` |
| 225.3 | 225.2 | -0.0 | `library/std/src/sys/pal/unix/time.rs:107` |
| 218.7 | 189.6 | -29.0 | `library/core/src/cell.rs:558` |
| 218.4 | 195.9 | -22.5 | `library/core/src/mem/mod.rs:976` |
| 216.0 | 216.0 | -0.0 | `./malloc/./malloc/malloc.c:4518` |
| 176.0 | 176.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/nix-0.31.3/src/sys/socket/mod.rs:2208` |
| 165.9 | 158.9 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 158.2 | 140.7 | -17.5 | `library/core/src/sync/atomic.rs:4104` |
| 158.1 | 152.9 | -5.2 | `library/core/src/sync/atomic.rs:3995` |
| 156.6 | 156.6 | -0.0 | `???:0` |
| 153.6 | 153.6 | 0.0 | `library/core/src/sync/atomic.rs:4013` |
| 151.5 | 153.7 | 2.2 | `library/core/src/ptr/non_null.rs:454` |
| 148.4 | 143.1 | -5.3 | `library/core/src/option.rs:743` |
| 142.9 | 129.4 | -13.5 | `library/std/src/sys/thread_local/native/mod.rs:68` |
| 141.5 | 129.7 | -11.7 | `library/core/src/cell.rs:2437` |
| 135.0 | 135.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 134.0 | 127.0 | -7.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:154` |
| 130.7 | 130.7 | 0.0 | `library/std/src/sys/sync/mutex/futex.rs:90` |
| 129.0 | 129.0 | -0.0 | `library/core/src/sync/atomic.rs:4049` |
| 118.0 | 118.0 | -0.0 | `/wt/benchmarks/counts/src/bin/counts.rs:21` |
| 108.0 | 108.0 | -0.0 | `./malloc/./malloc/malloc.c:4495` |
| 108.0 | 108.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 99.7 | 64.7 | -35.1 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/task/atomic_waker.rs:171` |
| 99.2 | 95.6 | -3.5 | `library/core/src/mem/mod.rs:975` |
| 98.0 | 98.0 | 0.0 | `library/core/src/slice/cmp.rs:157` |
| 97.0 | 97.0 | 0.0 | `library/core/src/result.rs:2177` |
| 94.1 | 94.1 | -0.0 | `library/core/src/ptr/mod.rs:1755` |
| 94.0 | 94.0 | 0.0 | `crates/bacnet-transport/src/bip/ingress.rs:105` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:4658` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3359` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3402` |
| 90.0 | 90.0 | -0.0 | `./malloc/./malloc/malloc.c:3392` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 90.0 | 90.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 90.0 | 90.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 88.0 | 88.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 87.0 | 87.0 | -0.0 | `./malloc/./malloc/malloc.c:1336` |
| 82.0 | 82.0 | 0.0 | `./socket/../sysdeps/unix/sysv/linux/recvmsg.c:27` |
| 81.0 | 81.0 | 0.0 | `library/core/src/option.rs:2439` |
| 79.5 | 73.1 | -6.5 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:684` |
| 79.0 | 79.0 | -0.0 | `library/core/src/ptr/mod.rs:1963` |
| 77.6 | 54.0 | -23.5 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/list.rs:326` |
| 75.0 | 75.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 74.6 | 56.0 | -18.5 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/sync/mpsc/chan.rs:318` |
| 74.2 | 74.2 | -0.0 | `library/core/src/option.rs:0` |
| 74.0 | 74.0 | 0.0 | `crates/bacnet-network/src/layer_receive.rs:89` |
| 74.0 | 65.0 | -9.0 | `library/std/src/sys/thread_local/native/eager.rs:35` |
| 73.3 | 50.1 | -23.2 | `index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/macros/select.rs:686` |
| 72.9 | 72.9 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:208` |
| 72.0 | 72.0 | -0.0 | `./malloc/./malloc/malloc.c:3368` |
