# encode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 156.0 | 147.0 | -9.0 | 2.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 128.0 | 112.0 | -16.0 | 7.00 | `__GI_memcpy` |
| 98.0 | 49.0 | -49.0 | 2.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 54.0 | 76.0 | 22.0 | 3.00 | `bacnet_services::read_property::encode_reference` |
| 47.0 | 47.0 | 0.0 | 1.00 | `free` |
| 42.0 | 0.0 | -42.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 26.0 | 26.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 24.0 | 24.0 | 0.0 | 1.00 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 16.0 | 16.0 | 0.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 15.0 | 15.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 6.0 | 6.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__rustc::__rust_dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `dl_main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getline` |
| 0.0 | 0.0 | 0.0 | 0.00 | `search_cache` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fillin_rpath.isra.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_no_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000650` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__new_exitfn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rtld_malloc_init_real` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000bc40` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 98.0 | 49.0 | -49.0 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 42.0 | 0.0 | -42.0 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 54.0 | 76.0 | 22.0 | `bacnet_services::read_property::encode_reference` |
| 128.0 | 112.0 | -16.0 | `__GI_memcpy` |
| 156.0 | 147.0 | -9.0 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 0.0 | 0.0 | 0.0 | `strstr` |
| 0.0 | 0.0 | 0.0 | `__call_tls_dtors` |
| 10.0 | 10.0 | 0.0 | `__aarch64_ldadd8_relax` |
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
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 96.0 | 84.0 | -12.0 | `library/core/src/ptr/mod.rs:573` |
| 20.0 | 13.0 | -7.0 | `crates/bacnet-encoding/src/tags/encode.rs:65` |
| 16.0 | 8.0 | -8.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:514` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:174` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:173` |
| 14.0 | 10.0 | -4.0 | `library/core/src/result.rs:2177` |
| 12.0 | 6.0 | -6.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:527` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 11.0 | 11.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 10.0 | 9.0 | -1.0 | `crates/bacnet-services/src/read_property.rs:33` |
| 10.0 | 13.0 | 3.0 | `crates/bacnet-encoding/src/tags/encode.rs:151` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 9.0 | 9.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 8.0 | 4.0 | -4.0 | `crates/bacnet-services/src/lib.rs:0` |
| 8.0 | 8.0 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:497` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:49` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:74` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 7.0 | 6.0 | -1.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 6.0 | 1.0 | -5.0 | `library/core/src/result.rs:0` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-services/src/who_is.rs:22` |
| 6.0 | 3.0 | -3.0 | `crates/bacnet-encoding/src/tags/encode.rs:64` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-services/src/who_is.rs:31` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 3.0 | -3.0 | `crates/bacnet-encoding/src/tags/encode.rs:59` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 1.0 | -4.0 | `crates/bacnet-types/src/primitives.rs:138` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:75` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-encoding/src/sink.rs:220` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:395` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-services/src/who_is.rs:29` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 5.0 | 5.0 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:87` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 4.0 | 2.0 | -2.0 | `crates/bacnet-encoding/src/tags/encode.rs:67` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:119` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
