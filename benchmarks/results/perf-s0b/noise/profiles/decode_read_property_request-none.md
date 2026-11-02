# decode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 71.0 | 71.0 | 0.0 | 0.00 | `codec_counts::run::{closure#2}` |
| 61.0 | 61.0 | 0.0 | 1.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 48.0 | 48.0 | 0.0 | 2.00 | `bacnet_encoding::tags::decode_tag` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_mmap_threshold` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sys::sync::once::futex::Once>::call` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_preload` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `dl_main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_runtime_resolve` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_find_object_from_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_lookup_direct` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `exit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_call_libc_early_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_scratch_buffer_set_array_size` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_unsave_markers` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_assign_tls_modid` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dlfo_sort_mappings` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigjmp_save` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memcpy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_acq` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::fmt::float::float_to_decimal_common_exact::<f64>` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___tunable_set_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_map_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_storage` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000600` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_hwcaps_split` |
| 0.0 | 0.0 | 0.0 | 0.00 | `rtld_mutex_dummy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_mmap_threshold` |
| 0.0 | 0.0 | 0.0 | `<std::sys::sync::once::futex::Once>::call` |
| 61.0 | 61.0 | 0.0 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 0.0 | 0.0 | 0.0 | `do_preload` |
| 0.0 | 0.0 | 0.0 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | `dl_main` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.0 | 0.0 | 0.0 | `_dl_runtime_resolve` |
| 0.0 | 0.0 | 0.0 | `calloc` |
| 0.0 | 0.0 | 0.0 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | `_int_malloc` |
| 0.0 | 0.0 | 0.0 | `_dl_find_object_from_map` |
| 0.0 | 0.0 | 0.0 | `_dl_lookup_direct` |
| 0.0 | 0.0 | 0.0 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | `exit` |
| 0.0 | 0.0 | 0.0 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | `_dl_call_libc_early_init` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `__libc_scratch_buffer_set_array_size` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 13.0 | 13.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 13.0 | 13.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:125` |
| 11.0 | 11.0 | 0.0 | `library/core/src/result.rs:2177` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:99` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:83` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:773` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:246` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:782` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:84` |
| 4.0 | 4.0 | 0.0 | `library/core/src/hint.rs:491` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:781` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:407` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 4.0 | 4.0 | 0.0 | `library/core/src/convert/num.rs:303` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:184` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:94` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 4.0 | 4.0 | 0.0 | `library/core/src/result.rs:0` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:97` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:157` |
| 2.0 | 2.0 | 0.0 | `library/core/src/num/uint_macros.rs:1096` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:256` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:137` |
| 2.0 | 2.0 | 0.0 | `library/core/src/result.rs:1183` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:128` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:103` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:132` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:141` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:17` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:527` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:106` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:148` |
| 1.0 | 1.0 | 0.0 | `library/core/src/result.rs:965` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:150` |
| 1.0 | 1.0 | 0.0 | `library/core/src/slice/index.rs:90` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:0` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/non_null.rs:1663` |
| 1.0 | 1.0 | 0.0 | `library/core/src/num/uint_macros.rs:743` |
| 1.0 | 1.0 | 0.0 | `library/core/src/result.rs:2178` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:105` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:147` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:149` |
| 1.0 | 1.0 | 0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:760` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
