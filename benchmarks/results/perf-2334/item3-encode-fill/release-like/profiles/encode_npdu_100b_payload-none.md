# encode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 155.0 | 131.0 | -24.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 37.0 | 37.0 | 0.0 | 1.00 | `free` |
| 31.0 | 31.0 | 0.0 | 1.00 | `codec_counts::run::{closure#4}` |
| 25.0 | 25.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 21.0 | 0.0 | -21.0 | 1.00 | `__GI_memset` |
| 20.0 | 40.0 | 20.0 | 1.00 | `__GI_memcpy` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sys::sync::once::futex::Once>::call` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_scratch_buffer_set_array_size` |
| 0.0 | 0.0 | 0.0 | 0.00 | `realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigsetjmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fstatat` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dlfo_process_initial` |
| 0.0 | 0.0 | 0.0 | 0.00 | `brk` |
| 0.0 | 0.0 | 0.0 | 0.00 | `version_check_doit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strsep` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sys::stdio::unix::Stderr as core::io::write::Write>::write_all` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_strtoul` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_name_match_p` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_trim_threshold` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__errno_location` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getline` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_default_finish` |
| 0.0 | 0.0 | 0.0 | 0.00 | `stat` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_cleanup_pop_restore` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_open` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::io::write::default_write_fmt::Adapter<std::io::stdio::StderrLock> as core::fmt::Write>::write_str` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_lookup_symbol_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_relax` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000001b580` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_underflow@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__cxa_atexit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_find_object_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_attr_destroy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_parse_arguments` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_read_whole_file` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 155.0 | 131.0 | -24.0 | `bacnet_encoding::npdu::encode_npdu` |
| 21.0 | 0.0 | -21.0 | `__GI_memset` |
| 20.0 | 40.0 | 20.0 | `__GI_memcpy` |
| 0.0 | 0.0 | 0.0 | `<std::sys::sync::once::futex::Once>::call` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | `__libc_scratch_buffer_set_array_size` |
| 1.0 | 1.0 | 0.0 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | `realloc` |
| 0.0 | 0.0 | 0.0 | `__sigsetjmp` |
| 0.0 | 0.0 | 0.0 | `fstatat` |
| 0.0 | 0.0 | 0.0 | `_dlfo_process_initial` |
| 0.0 | 0.0 | 0.0 | `brk` |
| 0.0 | 0.0 | 0.0 | `version_check_doit` |
| 0.0 | 0.0 | 0.0 | `strsep` |
| 0.0 | 0.0 | 0.0 | `_IO_file_close` |
| 0.0 | 0.0 | 0.0 | `<std::sys::stdio::unix::Stderr as core::io::write::Write>::write_all` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `_dl_strtoul` |
| 0.0 | 0.0 | 0.0 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | `_dl_name_match_p` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_trim_threshold` |
| 5.0 | 5.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 0.0 | 0.0 | 0.0 | `_dl_sort_maps` |
| 0.0 | 0.0 | 0.0 | `__errno_location` |
| 0.0 | 0.0 | 0.0 | `getline` |
| 0.0 | 0.0 | 0.0 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | `_IO_default_finish` |
| 0.0 | 0.0 | 0.0 | `stat` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 69.0 | 0.0 | -69.0 | `library/core/src/ptr/mod.rs:735` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 10.0 | 10.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 9.0 | 4.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 9.0 | 4.0 | -5.0 | `crates/bacnet-npdu/src/encode.rs:57` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 1.0 | -7.0 | `crates/bacnet-encoding/src/npdu.rs:174` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:163` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 6.0 | 7.0 | 1.0 | `library/core/src/option.rs:1227` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `library/core/src/option.rs:743` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 6.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:139` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 0.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:226` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:52` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:141` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:143` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 3.0 | 2.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:612` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:94` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 3.0 | 4.0 | 1.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 2.0 | 2.0 | 0.0 | `library/core/src/hint.rs:491` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:4517` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3182` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:4506` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3318` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3375` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:4499` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3190` |
| 2.0 | 2.0 | 0.0 | `library/core/src/option.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:132` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1225` |
