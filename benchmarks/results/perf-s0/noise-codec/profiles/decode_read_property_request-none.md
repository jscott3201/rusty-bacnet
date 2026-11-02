# decode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 101.0 | 101.0 | 0.0 | 2.00 | `codec_counts::run::{closure#2}` |
| 48.0 | 48.0 | 0.0 | 2.00 | `bacnet_encoding::tags::decode_tag` |
| 10.0 | 10.0 | 0.0 | 1.00 | `<bacnet_types::primitives::ObjectIdentifier>::decode` |
| 10.0 | 10.0 | 0.0 | 1.00 | `bacnet_encoding::primitives::decode_unsigned` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `map_doit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI__IO_un_link.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_mutex_unlock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_finish@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_ldadd8_relax` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_call_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::pal::unix::stack_overflow::thread_info::set_current_info` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::main` |

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
| 0.0 | 0.0 | 0.0 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | `map_doit` |
| 0.0 | 0.0 | 0.0 | `strcspn` |
| 0.0 | 0.0 | 0.0 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 19.0 | 19.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:125` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 14.0 | 14.0 | 0.0 | `library/core/src/result.rs:2177` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:83` |
| 6.0 | 6.0 | 0.0 | `library/core/src/result.rs:0` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:246` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:94` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:184` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:407` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 4.0 | 4.0 | 0.0 | `library/core/src/convert/num.rs:303` |
| 4.0 | 4.0 | 0.0 | `library/core/src/hint.rs:491` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:108` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:97` |
| 2.0 | 2.0 | 0.0 | `library/core/src/result.rs:1183` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:128` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:780` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:132` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:527` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:103` |
| 2.0 | 2.0 | 0.0 | `library/core/src/num/uint_macros.rs:1096` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:150` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:90` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:592` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:141` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:256` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:157` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:137` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:109` |
| 1.0 | 1.0 | 0.0 | `library/core/src/result.rs:965` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:760` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:148` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:154` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:0` |
| 1.0 | 1.0 | 0.0 | `library/core/src/num/uint_macros.rs:743` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:105` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:147` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:149` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:67` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:62` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:64` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:63` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
