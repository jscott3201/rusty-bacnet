# encode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 85.0 | 85.0 | 0.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 56.0 | 56.0 | 0.0 | 3.00 | `__GI_memcpy` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 45.0 | 45.0 | 0.0 | 1.00 | `free` |
| 25.0 | 25.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::pal::unix::stack_overflow::imp::make_handler` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas4_acq` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_new_file_init_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_find_object_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getline` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::num::imp::flt2dec::strategy::dragon::mul_pow10` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__internal_atexit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__glibc_morecore` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__cxa_atexit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_no_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_hwcaps_split` |
| 0.0 | 0.0 | 0.0 | 0.00 | `rtld_mutex_dummy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `brk` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__fopen_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `version_check_doit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_unsave_markers` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__cxa_finalize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_mmap_threshold` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | 0.00 | `call_weak_fn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_state` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000048504f8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_read_whole_file` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_setjmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000009690` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tunable_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memmove` |
| 0.0 | 0.0 | 0.0 | 0.00 | `mprotect` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::fmt::write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_hwcaps_subdirs_active` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_relax` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `std::sys::pal::unix::stack_overflow::imp::make_handler` |
| 0.0 | 0.0 | 0.0 | `memset` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | `__aarch64_cas4_acq` |
| 0.0 | 0.0 | 0.0 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | `_IO_new_file_init_internal` |
| 0.0 | 0.0 | 0.0 | `_dl_find_object_init` |
| 0.0 | 0.0 | 0.0 | `_dl_map_object` |
| 0.0 | 0.0 | 0.0 | `getline` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | `core::num::imp::flt2dec::strategy::dragon::mul_pow10` |
| 0.0 | 0.0 | 0.0 | `__internal_atexit` |
| 0.0 | 0.0 | 0.0 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | `__glibc_morecore` |
| 0.0 | 0.0 | 0.0 | `__GI_strlen` |
| 85.0 | 85.0 | 0.0 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 0.0 | 0.0 | 0.0 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | `__cxa_atexit` |
| 0.0 | 0.0 | 0.0 | `_IO_no_init` |
| 1.0 | 1.0 | 0.0 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | `_dl_hwcaps_split` |
| 0.0 | 0.0 | 0.0 | `rtld_mutex_dummy` |
| 0.0 | 0.0 | 0.0 | `brk` |
| 0.0 | 0.0 | 0.0 | `__fopen_internal` |
| 0.0 | 0.0 | 0.0 | `version_check_doit` |
| 0.0 | 0.0 | 0.0 | `_IO_unsave_markers` |
| 54.0 | 54.0 | 0.0 | `malloc` |
| 0.0 | 0.0 | 0.0 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `__cxa_finalize` |
| 0.0 | 0.0 | 0.0 | `_IO_cleanup` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 43.0 | 43.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 10.0 | 10.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:87` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:74` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:149` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:78` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:111` |
| 3.0 | 3.0 | 0.0 | `library/core/src/num/uint_macros.rs:743` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 3.0 | 3.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3318` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:4499` |
| 2.0 | 2.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:122` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1225` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3375` |
| 2.0 | 2.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:113` |
| 2.0 | 2.0 | 0.0 | `./malloc/./malloc/malloc.c:3190` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:0` |
| 2.0 | 2.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:112` |
