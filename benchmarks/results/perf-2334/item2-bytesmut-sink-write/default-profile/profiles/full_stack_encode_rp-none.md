# full_stack_encode_rp / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 325.0 | 325.0 | 0.0 | 5.00 | `_int_free` |
| 270.0 | 270.0 | 0.0 | 5.00 | `malloc` |
| 270.0 | 254.0 | -16.0 | 15.00 | `__GI_memcpy` |
| 188.0 | 188.0 | 0.0 | 5.00 | `free` |
| 171.0 | 171.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 156.0 | 147.0 | -9.0 | 2.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 130.0 | 130.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 98.0 | 49.0 | -49.0 | 2.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 94.0 | 94.0 | 0.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 88.0 | 88.0 | 0.0 | 3.00 | `codec_counts::run::{closure#12}` |
| 69.0 | 69.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 54.0 | 76.0 | 22.0 | 3.00 | `bacnet_services::read_property::encode_reference` |
| 52.0 | 52.0 | 0.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 50.0 | 50.0 | 0.0 | 10.00 | `__aarch64_ldadd8_relax` |
| 46.0 | 46.0 | 0.0 | 2.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 45.0 | 45.0 | 0.0 | 3.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 42.0 | 0.0 | -42.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| -0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strstr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_open` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_find_object_from_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::num::imp::bignum::Big32x40>::mul_pow2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_unload_cache` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<f64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | 0.00 | `parse_tunables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `openaux` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_setb` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fstatat` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000600` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rtld_malloc_init_stubs` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 98.0 | 49.0 | -49.0 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 42.0 | 0.0 | -42.0 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 54.0 | 76.0 | 22.0 | `bacnet_services::read_property::encode_reference` |
| 270.0 | 254.0 | -16.0 | `__GI_memcpy` |
| 156.0 | 147.0 | -9.0 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| -0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `strstr` |
| 0.0 | 0.0 | 0.0 | `__call_tls_dtors` |
| 50.0 | 50.0 | 0.0 | `__aarch64_ldadd8_relax` |
| 0.0 | 0.0 | 0.0 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `_IO_file_open` |
| 0.0 | 0.0 | 0.0 | `_dl_find_object_from_map` |
| 0.0 | 0.0 | 0.0 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | `<core::num::imp::bignum::Big32x40>::mul_pow2` |
| 0.0 | 0.0 | 0.0 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `_dl_unload_cache` |
| 0.0 | 0.0 | 0.0 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | `memchr` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | `<f64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | `parse_tunables` |
| 0.0 | 0.0 | 0.0 | `openaux` |
| 0.0 | 0.0 | 0.0 | `_IO_setb` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `_dl_sort_maps_init` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 201.0 | 189.0 | -12.0 | `library/core/src/ptr/mod.rs:573` |
| 60.0 | 60.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 55.0 | 55.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 52.0 | 52.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 45.0 | 45.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 40.0 | 40.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 25.0 | 25.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 25.0 | 25.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 22.0 | 19.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 20.0 | 13.0 | -7.0 | `crates/bacnet-encoding/src/tags/encode.rs:65` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 20.0 | 20.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 16.0 | 8.0 | -8.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:514` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:174` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:173` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 15.0 | 14.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:52` |
| 14.0 | 10.0 | -4.0 | `library/core/src/result.rs:2177` |
| 14.0 | 14.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 13.0 | 13.0 | 0.0 | `library/core/src/ptr/mod.rs:735` |
| 13.0 | 12.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 12.0 | 6.0 | -6.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:527` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:57` |
| 12.0 | 12.0 | 0.0 | `library/core/src/option.rs:1227` |
| 12.0 | 12.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:338` |
