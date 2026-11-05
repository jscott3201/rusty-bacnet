| Policy | Runtime | Instructions / request | Range of 6 | With counting allocator | Allocator calls | Bytes | Wall ns / request |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| none | ct | 39,656.3 | 13.5 | 40,709.3 | 25.0002 | 7,944.02 | 5,750 |
| none | mt | 40,830.6 | 46.5 | 41,878.6 | 25.0002 | 7,944.02 | 17,189 |
| pool | ct | 45,368.6 | 286.8 | 46,645.6 | 30.0008 | 8,406.04 | 18,112 |
| pool | mt | 47,465.2 | 323.1 | 48,737.3 | 30.0003 | 8,406.04 | 38,008 |
| inline | ct | 41,416.3 | 95.4 | 42,628.3 | 29.0002 | 8,150.02 | 5,869 |
| inline | mt | 42,624.8 | 122.4 | 43,831.8 | 29.0002 | 8,150.02 | 17,265 |

Calls per request, pool minus inline, ct:

- +35.01 `__aarch64_cas4_acq`
- +13.00 `__aarch64_swp4_rel`
- +11.00 `__pthread_disable_asynccancel`
- +11.00 `__pthread_enable_asynccancel`
- +10.01 `__aarch64_ldadd8_relax`
- +6.22 `__aarch64_cas8_rel`
- +6.02 `__aarch64_cas8_acq_rel`
- +5.01 `<std::sys::pal::unix::time::Timespec>::now`
- +5.01 `clock_gettime@@GLIBC_2.17`
- +4.22 `__aarch64_cas8_acq`
- +4.01 `__aarch64_ldadd8_acq_rel`
- +4.00 `core::ptr::drop_glue::<tokio::runtime::task::core::Stage<tokio::runtime::blocking::task::BlockingTask<bacnet_server::server::requests::mutations::ask_mutation_policy::{closure#0}::{closure#0}>>>`
- +3.01 `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- +3.00 `<bacnet_server::server::requests::mutations::Request>::decide::{closure#0}`
- -2.20 `__aarch64_swp8_acq`
- +2.04 `policy_counts::main`
- +2.00 `<tokio::runtime::time::Driver>::park_internal`
- +2.00 `<tokio::runtime::scheduler::inject::Inject<alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>>::pop`
- +2.00 `malloc`
- +2.00 `syscall`
- +2.00 `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}`
- +2.00 `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}, core::pin::Pin<alloc::boxed::Box<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>>>, <bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::dispatch::{closure#0}::{closure#1}>::{closure#0}`
- +2.00 `<std::sys::sync::condvar::futex::Condvar>::wait_timeout`
- +2.00 `tokio::runtime::task::harness::can_read_output`
- +2.00 `tokio::runtime::task::raw::poll::<<bacnet_server::server::request_tasks::RequestTasks>::try_spawn<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}, core::pin::Pin<alloc::boxed::Box<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>>>, <bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::dispatch::{closure#0}::{closure#1}>::{closure#0}, alloc::sync::Arc<tokio::runtime::scheduler::current_thread::Handle>>`

Calls per request, pool minus inline, mt:

- +24.32 `__aarch64_cas4_acq`
- +20.24 `__aarch64_swp4_rel`
- +11.00 `__aarch64_cas8_acq_rel`
- +10.21 `clock_gettime@@GLIBC_2.17`
- +10.02 `__aarch64_ldadd8_relax`
- +8.34 `__aarch64_ldadd8_acq_rel`
- +6.10 `<std::sys::pal::unix::time::Timespec>::now`
- +5.12 `<std::sys::pal::unix::time::Timespec>::sub_timespec`
- +4.34 `tokio::runtime::task::raw::poll::<tokio::runtime::blocking::task::BlockingTask<<tokio::runtime::scheduler::multi_thread::worker::Launch>::launch::{closure#0}>, tokio::runtime::blocking::schedule::BlockingSchedule>`
- +4.19 `syscall`
- +4.00 `core::ptr::drop_glue::<tokio::runtime::task::core::Stage<tokio::runtime::blocking::task::BlockingTask<bacnet_server::server::requests::mutations::ask_mutation_policy::{closure#0}::{closure#0}>>>`
- +3.99 `__aarch64_ldset8_acq_rel`
- +3.03 `<tokio::runtime::scheduler::multi_thread::worker::Context>::park_internal`
- +3.00 `<bacnet_server::server::requests::mutations::Request>::decide::{closure#0}`
- +2.99 `__aarch64_swp8_acq_rel`
- +2.07 `<tokio::runtime::time::Driver>::park_internal`
- +2.05 `__aarch64_cas1_acq_rel`
- +2.04 `<tokio::runtime::scheduler::multi_thread::worker::Core>::maintenance`
- +2.04 `__pthread_disable_asynccancel`
- +2.04 `__pthread_enable_asynccancel`
- +2.02 `__aarch64_ldadd8_rel`
- +2.01 `<tokio::runtime::scheduler::multi_thread::worker::Context>::run_task`
- +2.00 `<std::sys::sync::condvar::futex::Condvar>::wait_timeout`
- +2.00 `<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_confirmed_request::{closure#0}`
- +2.00 `<bacnet_server::server::request_tasks::RequestTasks>::try_spawn::<futures_util::future::either::Either<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::handle_admitted_read_property::{closure#0}, core::pin::Pin<alloc::boxed::Box<<bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::run_admitted_confirmed_request::{closure#0}>>>, <bacnet_server::server::BACnetServer<bacnet_transport::bip::BipTransport>>::dispatch::{closure#0}::{closure#1}>::{closure#0}`
