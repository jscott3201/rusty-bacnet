# full_stack_encode_rp / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 425.0 | 171.0 | -254.0 | 5.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 325.0 | 325.0 | 0.0 | 5.00 | `_int_free` |
| 270.0 | 270.0 | 0.0 | 5.00 | `malloc` |
| 258.0 | 270.0 | 12.0 | 14.00 | `__GI_memcpy` |
| 188.0 | 188.0 | 0.0 | 5.00 | `free` |
| 130.0 | 130.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 88.0 | 88.0 | 0.0 | 3.00 | `codec_counts::run::{closure#12}` |
| 78.0 | 69.0 | -9.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 50.0 | 50.0 | 0.0 | 10.00 | `__aarch64_ldadd8_relax` |
| 46.0 | 46.0 | 0.0 | 2.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 45.0 | 45.0 | 0.0 | 3.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 31.0 | 34.0 | 3.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 30.0 | 30.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 30.0 | 0.0 | -30.0 | 1.00 | `bacnet_encoding::tags::encode_tag` |
| 30.0 | 30.0 | 0.0 | 2.00 | `bytes::bytes::promotable_even_drop` |
| 27.0 | 52.0 | 25.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 27.0 | 0.0 | -27.0 | 1.00 | `bacnet_encoding::primitives::encode_ctx_unsigned` |
| 25.0 | 25.0 | 0.0 | 5.00 | `__rustc::__rust_dealloc` |
| 23.0 | 0.0 | -23.0 | 1.00 | `bacnet_encoding::primitives::encode_ctx_object_id` |
| 14.0 | 0.0 | -14.0 | 1.00 | `<bacnet_types::primitives::ObjectIdentifier>::encode` |
| 5.0 | 5.0 | 0.0 | 5.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
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

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 425.0 | 171.0 | -254.0 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 0.0 | 156.0 | 156.0 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 0.0 | 98.0 | 98.0 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 0.0 | 94.0 | 94.0 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 0.0 | 54.0 | 54.0 | `bacnet_services::read_property::encode_reference` |
| 0.0 | 42.0 | 42.0 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 0.0 | 37.0 | 37.0 | `<bytes::bytes_mut::BytesMut>::resize` |
| 30.0 | 0.0 | -30.0 | `bacnet_encoding::tags::encode_tag` |
| 27.0 | 0.0 | -27.0 | `bacnet_encoding::primitives::encode_ctx_unsigned` |
| 27.0 | 52.0 | 25.0 | `bacnet_encoding::npdu::encode_npdu` |
| 0.0 | 24.0 | 24.0 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 23.0 | 0.0 | -23.0 | `bacnet_encoding::primitives::encode_ctx_object_id` |
| 0.0 | 22.0 | 22.0 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 14.0 | 0.0 | -14.0 | `<bacnet_types::primitives::ObjectIdentifier>::encode` |
| 0.0 | 12.0 | 12.0 | `__GI_memset` |
| 258.0 | 270.0 | 12.0 | `__GI_memcpy` |
| 78.0 | 69.0 | -9.0 | `bacnet_encoding::apdu::encode_apdu` |
| 31.0 | 34.0 | 3.0 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_parse_arguments` |
| 0.0 | 0.0 | 0.0 | `write` |
| 0.0 | 0.0 | 0.0 | `strdup` |
| 325.0 | 325.0 | 0.0 | `_int_free` |
| 5.0 | 5.0 | 0.0 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | `__pthread_mutex_unlock_usercnt` |
| 0.0 | 0.0 | 0.0 | `_dl_important_hwcaps` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | `alloc_perturb` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 198.0 | 201.0 | 3.0 | `library/core/src/ptr/mod.rs:573` |
| 60.0 | 60.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 60.0 | 25.0 | -35.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 55.0 | 55.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 52.0 | 52.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 45.0 | 45.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 40.0 | 40.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 36.0 | 22.0 | -14.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 31.0 | 12.0 | -19.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 25.0 | 25.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 24.0 | 14.0 | -10.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 20.0 | 20.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 14.0 | 15.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/apdu/mod.rs:335` |
| 12.0 | 11.0 | -1.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 12.0 | 13.0 | 1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 12.0 | 7.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:612` |
| 12.0 | 7.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1208` |
| 11.0 | 11.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:54` |
| 11.0 | 11.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3317` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3165` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3178` |
| 10.0 | 10.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3167` |
