## Calls in per request, wp/mt: B - A (total over the 30 functions that moved by 0.5 or more: -10.9; the largest 25 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 3.00 | 0.00 | -3.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_tra` |
| 98.43 | 96.43 | -2.01 | `__aarch64_ldadd8_relax` |
| 0.00 | 2.00 | +2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_a` |
| 2.00 | 0.00 | -2.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_tra` |
| 0.00 | 2.00 | +2.00 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpor` |
| 2.00 | 0.00 | -2.00 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server:` |
| 21.00 | 20.00 | -1.00 | `malloc` |
| 24.00 | 23.00 | -1.00 | `_int_free` |
| 22.00 | 21.00 | -1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 21.00 | 20.00 | -1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 24.00 | 23.00 | -1.00 | `free` |
| 0.00 | 1.00 | +1.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_a` |
| 0.00 | 1.00 | +1.00 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_tr` |
| 1.00 | 0.00 | -1.00 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_s` |
| 50.00 | 49.00 | -1.00 | `__GI_memcpy` |
| 0.00 | 1.00 | +1.00 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_general::{closure#0}>` |
| 1.00 | 0.00 | -1.00 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>` |
| 0.00 | 1.00 | +1.00 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transport::bip::Bip` |
| 1.00 | 0.00 | -1.00 | `core::ptr::drop_glue::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BA` |
| 0.00 | 1.00 | +1.00 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnet` |
| 1.00 | 0.00 | -1.00 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::` |
| 0.00 | 1.00 | +1.00 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<ba` |
| 1.00 | 0.00 | -1.00 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<b` |
| 0.00 | 1.00 | +1.00 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServe` |
| 1.00 | 0.00 | -1.00 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Eithe` |

## Self instructions per request, wp/mt: B - A (total over the 46 functions that moved by 0.5 or more: -377.2; the largest 25 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 280.00 | 0.00 | -280.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_tra` |
| 0.00 | 259.00 | +259.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_a` |
| 0.00 | 135.00 | +135.00 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpor` |
| 135.00 | 0.00 | -135.00 | `<tokio::runtime::task::core::Core<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server:` |
| 1,621.00 | 1,495.00 | -126.00 | `__GI_memcpy` |
| 95.00 | 0.00 | -95.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_tra` |
| 0.00 | 91.00 | +91.00 | `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_a` |
| 1,618.24 | 1,554.99 | -63.25 | `_int_free` |
| 0.00 | 60.00 | +60.00 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_transpor` |
| 60.00 | 0.00 | -60.00 | `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server:` |
| 0.00 | 57.00 | +57.00 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServe` |
| 57.00 | 0.00 | -57.00 | `tokio::runtime::task::raw::drop_join_handle_slow::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Eithe` |
| 1,139.14 | 1,091.10 | -48.05 | `malloc` |
| 0.00 | 44.00 | +44.00 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacnet_tr` |
| 44.00 | 0.00 | -44.00 | `<tokio::runtime::task::harness::Harness<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_s` |
| 0.00 | 41.00 | +41.00 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<bacn` |
| 41.00 | 0.00 | -41.00 | `tokio::runtime::task::raw::try_read_output::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bac` |
| 1,194.67 | 1,160.95 | -33.71 | `free` |
| 61.16 | 34.97 | -26.19 | `_int_free_maybe_consolidate.part.0` |
| 607.02 | 581.01 | -26.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.00 | 22.00 | +22.00 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnet` |
| 22.00 | 0.00 | -22.00 | `core::ptr::drop_glue::<tokio::runtime::task::core::Cell<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::` |
| 0.00 | 14.00 | +14.00 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_general::{closure#0}>` |
| 14.00 | 0.00 | -14.00 | `core::ptr::drop_glue::<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>` |
| 0.00 | 14.00 | +14.00 | `tokio::runtime::task::raw::drop_abort_handle::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<<bacnet_server::server::BACnetServer<ba` |

