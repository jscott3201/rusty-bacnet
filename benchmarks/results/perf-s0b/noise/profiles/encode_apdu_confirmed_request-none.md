# encode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 110.0 | 110.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 37.0 | 37.0 | 0.0 | 1.00 | `free` |
| 31.0 | 31.0 | 0.0 | 1.00 | `codec_counts::run::{closure#8}` |
| 25.0 | 25.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 14.0 | 14.0 | 0.0 | 1.00 | `__GI_memcpy` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| -0.0 | -0.0 | 0.0 | 0.00 | `rindex` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_mmap_threshold` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::promotable_even_drop` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc_zeroed` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_map_versions` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_mmap_threshold` |
| 0.0 | 0.0 | 0.0 | `bytes::bytes::promotable_even_drop` |
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
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc_zeroed` |
| 0.0 | 0.0 | 0.0 | `__libc_scratch_buffer_set_array_size` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 34.0 | 34.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:338` |
| 10.0 | 10.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:337` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:128` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 6.0 | 6.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:164` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1281` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:166` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:168` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:226` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1208` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:94` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3190` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1225` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3317` |
| 2.0 | 2.0 | 0.0 | `library/core/src/hint.rs:491` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3165` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3178` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3167` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:382` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3182` |
| 2.0 | 2.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:167` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:4506` |
| 2.0 | 2.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:30` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:4517` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
