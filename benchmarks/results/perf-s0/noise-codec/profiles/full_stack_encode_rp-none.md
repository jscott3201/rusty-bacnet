# full_stack_encode_rp / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 325.0 | 325.0 | 0.0 | 5.00 | `_int_free` |
| 270.0 | 270.0 | 0.0 | 15.00 | `__GI_memcpy` |
| 270.0 | 270.0 | 0.0 | 5.00 | `malloc` |
| 188.0 | 188.0 | 0.0 | 5.00 | `free` |
| 171.0 | 171.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 156.0 | 156.0 | 0.0 | 2.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 130.0 | 130.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 98.0 | 98.0 | 0.0 | 2.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 94.0 | 94.0 | 0.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 88.0 | 88.0 | 0.0 | 3.00 | `codec_counts::run::{closure#12}` |
| 69.0 | 69.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 54.0 | 54.0 | 0.0 | 3.00 | `bacnet_services::read_property::encode_reference` |
| 52.0 | 52.0 | 0.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 50.0 | 50.0 | 0.0 | 10.00 | `__aarch64_ldadd8_relax` |
| 46.0 | 46.0 | 0.0 | 2.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 45.0 | 45.0 | 0.0 | 3.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 42.0 | 42.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 37.0 | 37.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 34.0 | 34.0 | 0.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 30.0 | 30.0 | 0.0 | 2.00 | `bytes::bytes::promotable_even_drop` |
| 30.0 | 30.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 25.0 | 25.0 | 0.0 | 5.00 | `__rustc::__rust_dealloc` |
| 24.0 | 24.0 | 0.0 | 1.00 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 22.0 | 22.0 | 0.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 12.0 | 12.0 | 0.0 | 1.00 | `__GI_memset` |
| 5.0 | 5.0 | 0.0 | 5.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
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

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | `__ctype_init` |
| 130.0 | 130.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
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
| 98.0 | 98.0 | 0.0 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 0.0 | 0.0 | 0.0 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `memmove` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `expand_dynamic_string_token` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 201.0 | 201.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 60.0 | 60.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 55.0 | 55.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 52.0 | 52.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 45.0 | 45.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 40.0 | 40.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 25.0 | 25.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 25.0 | 25.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 22.0 | 22.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 20.0 | 20.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:65` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 20.0 | 20.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:31` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:138` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:514` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:137` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 15.0 | 15.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:52` |
| 14.0 | 14.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 14.0 | 14.0 | 0.0 | `library/core/src/result.rs:2177` |
| 13.0 | 13.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 13.0 | 13.0 | 0.0 | `library/core/src/ptr/mod.rs:735` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:338` |
| 12.0 | 12.0 | 0.0 | `library/core/src/option.rs:1227` |
| 12.0 | 12.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:527` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:57` |
