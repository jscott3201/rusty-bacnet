# encode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 156.0 | 156.0 | 0.0 | 4.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 128.0 | 128.0 | 0.0 | 7.00 | `__GI_memcpy` |
| 98.0 | 98.0 | 0.0 | 2.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 65.0 | 65.0 | 0.0 | 2.00 | `_int_free` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 54.0 | 54.0 | 0.0 | 4.00 | `bacnet_services::read_property::encode_reference` |
| 47.0 | 47.0 | 0.0 | 2.00 | `free` |
| 42.0 | 42.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 26.0 | 26.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 24.0 | 24.0 | 0.0 | 1.00 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 16.0 | 16.0 | 0.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 15.0 | 15.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 6.0 | 6.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__rustc::__rust_dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::env::args` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rtld_malloc_init_stubs` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__tls_pre_init_tp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__unregister_atfork` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_preload` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::num::imp::flt2dec::digits_to_dec_str` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `map_doit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__cxa_finalize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_flush_all` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |
| 0.0 | 0.0 | 0.0 | 0.00 | `exit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_map_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__open_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | 0.00 | `index` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_update` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | `std::env::args` |
| 0.0 | 0.0 | 0.0 | `__rtld_malloc_init_stubs` |
| 0.0 | 0.0 | 0.0 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | `__tls_pre_init_tp` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | `__unregister_atfork` |
| 0.0 | 0.0 | 0.0 | `do_preload` |
| 0.0 | 0.0 | 0.0 | `core::num::imp::flt2dec::digits_to_dec_str` |
| 0.0 | 0.0 | 0.0 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `do_lookup_x` |
| 24.0 | 24.0 | 0.0 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 1.0 | 1.0 | 0.0 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | `map_doit` |
| 0.0 | 0.0 | 0.0 | `strcspn` |
| 0.0 | 0.0 | 0.0 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | `_dl_determine_tlsoffset` |
| 6.0 | 6.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 96.0 | 96.0 | 0.0 | `library/core/src/ptr/mod.rs:573` |
| 20.0 | 20.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:65` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:514` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:138` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:137` |
| 14.0 | 14.0 | 0.0 | `library/core/src/result.rs:2177` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:527` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 11.0 | 11.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 10.0 | 10.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:31` |
| 10.0 | 10.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:151` |
| 9.0 | 9.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `library/core/src/mem/mod.rs:975` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-services/src/lib.rs:0` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:497` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:72` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:47` |
| 7.0 | 7.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 6.0 | 6.0 | 0.0 | `library/core/src/result.rs:0` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-services/src/who_is.rs:31` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-services/src/who_is.rs:22` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:64` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:59` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:521` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:184` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/primitives/scalar.rs:395` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:135` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:85` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:73` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-services/src/who_is.rs:29` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:122` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:67` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:113` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
