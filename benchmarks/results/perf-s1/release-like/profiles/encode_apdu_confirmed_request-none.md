# encode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 206.0 | 0.0 | -206.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 94.0 | 14.0 | -80.0 | 5.00 | `__GI_memcpy` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 63.0 | 110.0 | 47.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 37.0 | 37.0 | 0.0 | 1.00 | `free` |
| 31.0 | 31.0 | 0.0 | 1.00 | `codec_counts::run::{closure#8}` |
| 25.0 | 25.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 1.0 | 0.0 | -1.0 | 1.00 | `codec_counts::run` |
| -0.0 | -0.0 | 0.0 | 0.00 | `rindex` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| -0.0 | 0.0 | 0.0 | 0.00 | `____strtoul_l_internal` |
| 0.0 | 1.0 | 1.0 | 0.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_match` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::num::imp::flt2dec::strategy::dragon::mul_pow10` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `exit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fclose@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000600` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__init_cpu_features` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__new_exitfn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_doallocate` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_state` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__read_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__vfscanf_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `open_verify.constprop.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fixup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000634` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000001b580` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_receive_error` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 206.0 | 0.0 | -206.0 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 94.0 | 14.0 | -80.0 | `__GI_memcpy` |
| 63.0 | 110.0 | 47.0 | `bacnet_encoding::apdu::encode_apdu` |
| 0.0 | 1.0 | 1.0 | `codec_counts::main` |
| 1.0 | 0.0 | -1.0 | `codec_counts::run` |
| -0.0 | 0.0 | 0.0 | `____strtoul_l_internal` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | `bcmp` |
| 0.0 | 0.0 | 0.0 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | `poll` |
| 0.0 | 0.0 | 0.0 | `check_match` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | `core::num::imp::flt2dec::strategy::dragon::mul_pow10` |
| 0.0 | 0.0 | 0.0 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | `exit` |
| 0.0 | 0.0 | 0.0 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | `fclose@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000600` |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | `__GI_memset` |
| 0.0 | 0.0 | 0.0 | `__init_cpu_features` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 90.0 | 3.0 | -87.0 | `library/core/src/ptr/mod.rs:573` |
| 40.0 | 0.0 | -40.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 15.0 | 6.0 | -9.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 15.0 | 1.0 | -14.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/apdu/mod.rs:335` |
| 12.0 | 0.0 | -12.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 11.0 | 34.0 | 23.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 10.0 | 10.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/apdu/mod.rs:152` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/apdu/mod.rs:135` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/apdu/mod.rs:334` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/apdu/mod.rs:357` |
| 6.0 | 6.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:164` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 5.0 | 0.0 | -5.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 5.0 | 3.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1208` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 0.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:612` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 5.0 | 1.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:113` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:122` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:115` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:117` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:119` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:166` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 4.0 | 0.0 | -4.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:114` |
