# encode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 171.0 | 171.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 94.0 | 94.0 | 0.0 | 5.00 | `__GI_memcpy` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 53.0 | 53.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 37.0 | 37.0 | 0.0 | 1.00 | `free` |
| 27.0 | 27.0 | 0.0 | 1.00 | `codec_counts::run::{closure#8}` |
| 26.0 | 26.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 15.0 | 15.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 6.0 | 6.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__rustc::__rust_dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__ctype_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::fmt::write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__tls_init_tp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_call_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_protect_relro` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sbrk` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `access` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memmove` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `expand_dynamic_string_token` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_old_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `dl_main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000036c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000650` |
| 0.0 | 0.0 | 0.0 | 0.00 | `stat` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__vfscanf_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000b890` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_catch_exception` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_relocate_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003830` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<str as core::fmt::Display>::fmt` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | `__ctype_init` |
| 26.0 | 26.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 0.0 | 0.0 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | `core::fmt::write` |
| 0.0 | 0.0 | 0.0 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | `__tls_init_tp` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `_dl_call_fini` |
| 0.0 | 0.0 | 0.0 | `_dl_protect_relro` |
| 0.0 | 0.0 | 0.0 | `sbrk` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `access` |
| 0.0 | 0.0 | 0.0 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `memmove` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `expand_dynamic_string_token` |
| 0.0 | 0.0 | 0.0 | `0x0000000004883190` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 75.0 | 75.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 25.0 | 25.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:338` |
| 12.0 | 12.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 11.0 | 11.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 11.0 | 11.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 10.0 | 10.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:128` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:337` |
| 7.0 | 7.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:164` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:360` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1208` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:612` |
| 5.0 | 5.0 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:118` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:120` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:122` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:113` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:115` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:117` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:119` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:166` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:114` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:31` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:168` |
