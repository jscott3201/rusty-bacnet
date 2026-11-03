# encode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 171.0 | 171.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 94.0 | 94.0 | 0.0 | 5.00 | `__GI_memcpy` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 63.0 | 53.0 | -10.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 37.0 | 37.0 | 0.0 | 1.00 | `free` |
| 27.0 | 27.0 | 0.0 | 1.00 | `codec_counts::run::{closure#8}` |
| 26.0 | 26.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 15.0 | 15.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 6.0 | 6.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__rustc::__rust_dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| -0.0 | -0.0 | 0.0 | 0.00 | `rindex` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_parse_arguments` |
| 0.0 | 0.0 | 0.0 | 0.00 | `write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strdup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_mutex_unlock_usercnt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_important_hwcaps` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strstr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_storage` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `access` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_default_finish` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `call_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_link_in` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `expand_dynamic_string_token` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000005c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | 0.00 | `rtld_mutex_dummy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__read_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__ctype_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `lookup_malloc_symbol` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 63.0 | 53.0 | -10.0 | `bacnet_encoding::apdu::encode_apdu` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_parse_arguments` |
| 0.0 | 0.0 | 0.0 | `write` |
| 0.0 | 0.0 | 0.0 | `strdup` |
| 65.0 | 65.0 | 0.0 | `_int_free` |
| 1.0 | 1.0 | 0.0 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | `__pthread_mutex_unlock_usercnt` |
| 0.0 | 0.0 | 0.0 | `_dl_important_hwcaps` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | `getauxval` |
| 6.0 | 6.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 0.0 | 0.0 | 0.0 | `strstr` |
| 0.0 | 0.0 | 0.0 | `_dl_start` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | `_dl_allocate_tls_storage` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start` |
| 0.0 | 0.0 | 0.0 | `access` |
| 0.0 | 0.0 | 0.0 | `_IO_default_finish` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | `call_init` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 75.0 | 75.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 25.0 | 25.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/apdu/mod.rs:335` |
| 12.0 | 12.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 11.0 | 11.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 11.0 | 11.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 10.0 | 10.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/apdu/mod.rs:152` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/apdu/mod.rs:135` |
| 7.0 | 7.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:164` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/apdu/mod.rs:357` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/apdu/mod.rs:334` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 5.0 | 5.0 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:612` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1208` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:122` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:113` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:115` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:117` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:119` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:114` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:166` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
