# full_stack_encode_rp / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 325.0 | 325.0 | 0.0 | 5.00 | `_int_free` |
| 270.0 | 270.0 | 0.0 | 5.00 | `malloc` |
| 196.0 | 196.0 | 0.0 | 5.00 | `free` |
| 156.0 | 156.0 | 0.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 126.0 | 126.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 125.0 | 125.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 118.0 | 118.0 | 0.0 | 7.00 | `__GI_memcpy` |
| 97.0 | 97.0 | 0.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 83.0 | 83.0 | 0.0 | 5.00 | `codec_counts::run::{closure#12}` |
| 50.0 | 50.0 | 0.0 | 10.00 | `__aarch64_ldadd8_relax` |
| 28.0 | 28.0 | 0.0 | 2.00 | `bytes::bytes::promotable_even_drop` |
| 25.0 | 25.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 12.0 | 12.0 | 0.0 | 1.00 | `__GI_memset` |
| 1.0 | 1.0 | 0.0 | 1.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_mmap_threshold` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sys::sync::once::futex::Once>::call` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_preload` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `dl_main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_runtime_resolve` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_find_object_from_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_lookup_direct` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `exit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_call_libc_early_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_scratch_buffer_set_array_size` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_unsave_markers` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_assign_tls_modid` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dlfo_sort_mappings` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigjmp_save` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memcpy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_acq` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::fmt::float::float_to_decimal_common_exact::<f64>` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___tunable_set_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038c0` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_mmap_threshold` |
| 28.0 | 28.0 | 0.0 | `bytes::bytes::promotable_even_drop` |
| 0.0 | 0.0 | 0.0 | `<std::sys::sync::once::futex::Once>::call` |
| 0.0 | 0.0 | 0.0 | `do_preload` |
| 0.0 | 0.0 | 0.0 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | `dl_main` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | `_dl_runtime_resolve` |
| 0.0 | 0.0 | 0.0 | `calloc` |
| 0.0 | 0.0 | 0.0 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | `_int_malloc` |
| 0.0 | 0.0 | 0.0 | `_dl_find_object_from_map` |
| 0.0 | 0.0 | 0.0 | `_dl_lookup_direct` |
| 0.0 | 0.0 | 0.0 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | `exit` |
| 0.0 | 0.0 | 0.0 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | `_dl_call_libc_early_init` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `__libc_scratch_buffer_set_array_size` |
| 0.0 | 0.0 | 0.0 | `_int_realloc` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 70.0 | 70.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 69.0 | 69.0 | 0.0 | `library/core/src/ptr/mod.rs:735` |
| 60.0 | 60.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 50.0 | 50.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 45.0 | 45.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 40.0 | 40.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 34.0 | 34.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 25.0 | 25.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:338` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3190` |
| 10.0 | 10.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:54` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3317` |
| 10.0 | 10.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:190` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3165` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3178` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3167` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3182` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:4506` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1262` |
| 10.0 | 10.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:30` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:4517` |
| 10.0 | 10.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3318` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:4499` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3375` |
| 9.0 | 9.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:196` |
| 9.0 | 9.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:57` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:337` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:128` |
| 8.0 | 8.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:203` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:226` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:0` |
